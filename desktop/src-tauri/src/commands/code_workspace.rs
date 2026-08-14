use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Manager, State};

use crate::app_state::AppState;
use crate::code_workspace::{
    code_thread_source, preflight_execution_root, revalidate_execution_root,
    CodeApprovalResponseInput, CodeBoundThreadOpenResult, CodeBoundThreadSummary, CodeEventBacklog,
    CodePreparedWorktree, CodeRecoveryThread, CodeRepositoryDescriptor, CodeRepositoryInspectInput,
    CodeRuntimeEvent, CodeRuntimeEventBacklog, CodeRuntimeProbe, CodeRuntimeStatus,
    CodeThreadBinding, CodeThreadBindingLookupInput, CodeThreadBindingRecoverInput,
    CodeThreadBindingScope, CodeThreadBindingStore, CodeThreadListInput, CodeThreadPreparation,
    CodeThreadPreparationListInput, CodeThreadResumeInput, CodeThreadStartError,
    CodeThreadStartInput, CodeThreadsPage, CodeTurnInterruptInput, CodeTurnStartInput,
    CodeTurnSteerInput, CodeTurnSummary, CodeWorkspaceEvent, CodeWorktreeDescriptor,
    CodeWorktreePrepareCommandInput, CodeWorktreeStatus, CODE_WORKSPACE_EVENT,
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
    let runtime = state.code_runtime.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock_bindings(&binding_lock)?;
        let store = CodeThreadBindingStore::for_app_data(&app_data_dir)?;
        let cache = Mutex::new(EventScopeCache::default());
        let emitter = Arc::new(move |event| {
            if let Some(event) = scope_live_event(&store, &cache, event) {
                let _ = app.emit(CODE_WORKSPACE_EVENT, event);
            }
        });
        runtime.start(emitter)
    })
    .await
    .map_err(|error| format!("Codex start task failed: {error}"))?
}

#[tauri::command]
/// Stop the whole app-server process group and invalidate pending approvals.
pub async fn code_runtime_stop(state: State<'_, AppState>) -> Result<CodeRuntimeStatus, String> {
    let runtime = state.code_runtime.clone();
    let binding_lock = state.code_thread_bindings_lock.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = lock_bindings(&binding_lock)?;
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
    let worktree = crate::code_workspace::prepare_execution_root(input.into_native(), nest_root)?;
    let preparation_id = uuid::Uuid::new_v4().hyphenated().to_string();
    store
        .create_preparation(preparation_id.clone(), scope.clone(), &worktree.descriptor)
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
/// List only Codex threads durably bound to one exact SchoolX project scope.
pub async fn code_threads_list(
    input: CodeThreadListInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<CodeThreadsPage, String> {
    let app_data_dir = code_app_data_dir(&app)?;
    let nest_root = code_nest_root()?;
    let runtime = state.code_runtime.clone();
    tauri::async_runtime::spawn_blocking(move || {
        list_threads_native(input, &app_data_dir, &nest_root, &runtime)
    })
    .await
    .map_err(|error| format!("Codex thread list task failed: {error}"))?
}

fn list_threads_native(
    input: CodeThreadListInput,
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
) -> Result<CodeThreadsPage, String> {
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    let bindings = store.list(&input.scope)?;
    let mut data = Vec::with_capacity(bindings.len());
    for binding in bindings {
        let hydrated = (|| {
            let execution_root = revalidate_binding_root(&binding, nest_root)?;
            let thread = runtime.thread_read(&binding.codex_thread_id)?;
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
                thread: Some(thread),
                unavailable: None,
            }),
            Err(error) => data.push(CodeBoundThreadSummary {
                binding,
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
    tauri::async_runtime::spawn_blocking(move || {
        start_thread_native(input, &app_data_dir, &nest_root, &runtime, &binding_lock)
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
        let binding =
            store.commit_preparation_binding(&preparation.scope(), &preparation_id, &thread_id)?;
        Ok(CodeBoundThreadOpenResult {
            binding,
            thread: opened.thread,
            instruction_sources: opened.instruction_sources,
        })
    })();

    commit_result.map_err(|error: String| {
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
    let binding_lock = state.code_thread_bindings_lock.clone();
    tauri::async_runtime::spawn_blocking(move || {
        recover_thread_binding_native(input, &app_data_dir, &nest_root, &runtime, &binding_lock)
    })
    .await
    .map_err(|error| format!("SchoolX Code binding recovery task failed: {error}"))?
}

fn recover_thread_binding_native(
    input: CodeThreadBindingRecoverInput,
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
    binding_lock: &Mutex<()>,
) -> Result<CodeBoundThreadOpenResult, String> {
    let _guard = lock_bindings(binding_lock)?;
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    let preparation = store.starting_preparation(&input.scope, &input.preparation_id)?;
    let execution_root = revalidate_execution_root(&preparation.descriptor(), nest_root)?
        .descriptor
        .execution_root;
    let candidates = runtime.recovery_threads_at(&execution_root)?;
    let bound_thread_ids = store
        .load()?
        .bindings
        .into_iter()
        .map(|binding| binding.codex_thread_id)
        .collect();
    let candidate =
        select_recovery_candidate(&preparation, candidates, &bound_thread_ids, &execution_root)?;
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
    let opened = runtime.thread_resume_at(resume, &execution_root)?;
    validate_recovery_source(
        &preparation,
        &CodeRecoveryThread {
            thread: opened.thread.clone(),
            thread_source: opened.thread_source.clone(),
        },
    )?;
    validate_thread_identity_and_root(
        &opened.thread.id,
        opened.thread.cwd.as_deref(),
        &thread_id,
        &execution_root,
    )?;
    let binding =
        store.commit_preparation_binding(&input.scope, &input.preparation_id, &thread_id)?;
    Ok(CodeBoundThreadOpenResult {
        binding,
        thread: opened.thread,
        instruction_sources: opened.instruction_sources,
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
    tauri::async_runtime::spawn_blocking(move || {
        resume_thread_native(input, &app_data_dir, &nest_root, &runtime)
    })
    .await
    .map_err(|error| format!("Codex thread resume task failed: {error}"))?
}

fn resume_thread_native(
    input: CodeThreadResumeInput,
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
) -> Result<CodeBoundThreadOpenResult, String> {
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    let binding = require_binding(&store, &input.scope, &input.thread_id)?;
    let execution_root = revalidate_binding_root(&binding, nest_root)?;
    let expected_thread_id = binding.codex_thread_id.clone();
    let opened = runtime.thread_resume_at(input, &execution_root)?;
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
    tauri::async_runtime::spawn_blocking(move || {
        start_turn_native(input, &app_data_dir, &nest_root, &runtime)
    })
    .await
    .map_err(|error| format!("Codex turn start task failed: {error}"))?
}

fn start_turn_native(
    input: CodeTurnStartInput,
    app_data_dir: &Path,
    nest_root: &Path,
    runtime: &crate::code_workspace::CodeRuntime,
) -> Result<CodeTurnSummary, String> {
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    let binding = require_binding(&store, &input.scope, &input.thread_id)?;
    let execution_root = revalidate_binding_root(&binding, nest_root)?;
    runtime.turn_start_at(input, &execution_root)
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
    tauri::async_runtime::spawn_blocking(move || {
        let store = CodeThreadBindingStore::for_app_data(&app_data_dir)?;
        let binding = require_binding(&store, &input.scope, &input.thread_id)?;
        revalidate_binding_root(&binding, &nest_root)?;
        runtime.turn_steer(input)
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
    tauri::async_runtime::spawn_blocking(move || {
        let store = CodeThreadBindingStore::for_app_data(&app_data_dir)?;
        require_binding(&store, &input.scope, &input.thread_id)?;
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
    tauri::async_runtime::spawn_blocking(move || {
        let store = CodeThreadBindingStore::for_app_data(&app_data_dir)?;
        let binding = require_binding(&store, &input.scope, &input.thread_id)?;
        if let Some(nest_root) = nest_root.as_deref() {
            revalidate_binding_root(&binding, nest_root)?;
        }
        runtime.approval_respond(input)
    })
    .await
    .map_err(|error| format!("Codex approval response task failed: {error}"))?
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
    bound_thread_ids: &HashSet<String>,
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
        validate_thread_identity_and_root(
            &candidate.thread.id,
            candidate.thread.cwd.as_deref(),
            &candidate.thread.id,
            expected_root,
        )?;
        if bound_thread_ids.contains(&candidate.thread.id) {
            continue;
        }
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
mod tests {
    use super::*;
    use crate::code_workspace::bindings::{CodeExecutionMode, CodeThreadPreparationState};
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
            state: crate::code_workspace::bindings::CodeThreadPreparationState::Starting,
            recovery_thread_baseline: baseline
                .map(|thread_ids| thread_ids.into_iter().map(str::to_string).collect()),
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
        }
    }

    #[cfg(unix)]
    struct FakeCodex {
        _directory: tempfile::TempDir,
        executable: PathBuf,
    }

    #[cfg(unix)]
    impl FakeCodex {
        fn created_marker(&self) -> PathBuf {
            self.executable.with_file_name("codex.created")
        }

        fn recorded_requests(&self) -> Result<Vec<serde_json::Value>, String> {
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
    fn stateful_fake_codex(
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
                "cwd": execution_root,
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
        let empty_loaded = shell_double_quoted_json(&json!({
            "data": [],
            "nextCursor": null
        }))?;
        let opened = shell_double_quoted_json(&json!({
            "thread": thread(thread_id),
            "instructionSources": []
        }))?;
        let read = shell_double_quoted_json(&json!({ "thread": thread(thread_id) }))?;
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
IFS= read -r initialize
printf '%s\n' '{{"id":1,"result":{{"userAgent":"codex-phase1c","codexHome":"/tmp/codex-home","platformFamily":"unix","platformOs":"macos"}}}}'
IFS= read -r initialized
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$0.requests"
  request_id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"thread/list"'*)
      if [ -f "$0.created" ]; then
        printf '%s\n' "{{\"id\":$request_id,\"result\":{recovery_page}}}"
      else
        printf '%s\n' "{{\"id\":$request_id,\"result\":{baseline_page}}}"
      fi
      ;;
    *'"method":"thread/loaded/list"'*)
      printf '%s\n' "{{\"id\":$request_id,\"result\":{empty_loaded}}}"
      ;;
    *'"method":"thread/start"'*)
      : > "$0.created"
      {start_reply}
      ;;
    *'"method":"thread/read"'*)
      printf '%s\n' "{{\"id\":$request_id,\"result\":{read}}}"
      ;;
    *'"method":"thread/resume"'*)
      printf '%s\n' "{{\"id\":$request_id,\"result\":{opened}}}"
      ;;
    *'"method":"turn/start"'*)
      printf '%s\n' "{{\"id\":$request_id,\"result\":{turn}}}"
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

    struct TestRepository {
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

    fn create_test_repository() -> Result<TestRepository, String> {
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

    fn phase1c_scope(repository_identity: String) -> CodeThreadBindingScope {
        CodeThreadBindingScope {
            community_id: "community-phase1c".to_string(),
            project_dtag: "project-phase1c".to_string(),
            repository_identity,
        }
    }

    fn method_count(requests: &[serde_json::Value], method: &str) -> usize {
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
        let backlog = CodeRuntimeEventBacklog {
            runtime_generation: 9,
            latest_sequence: 4,
            truncated: true,
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
            recovery_candidate("thread-bound", Some(&root), Some(&source)),
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
    fn native_binding_survives_process_restart_list_resume_and_turn() -> Result<(), String> {
        let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let repository = create_test_repository()?;
        let repository_root = repository.root.to_string_lossy().into_owned();
        let repository_descriptor = preflight_execution_root(&repository_root, "HEAD")?;
        let scope = phase1c_scope(repository_descriptor.repository_identity);
        let binding_lock = Mutex::new(());

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

        let page = list_threads_native(
            CodeThreadListInput {
                scope: scope.clone(),
            },
            app_data.path(),
            nest.path(),
            &runtime,
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
        )?;
        assert_eq!(turn.id, "turn-phase1c");
        runtime.stop()?;

        let requests = fake.recorded_requests()?;
        assert_eq!(method_count(&requests, "thread/list"), 1);
        assert_eq!(method_count(&requests, "thread/loaded/list"), 1);
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
        let recovered = recover_thread_binding_native(
            CodeThreadBindingRecoverInput {
                scope: scope.clone(),
                preparation_id: preparation_id.clone(),
                model: None,
            },
            app_data.path(),
            nest.path(),
            &second_runtime,
            &binding_lock,
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
