use super::*;
use crate::code_workspace::bindings::CodeExecutionMode;
#[cfg(unix)]
use crate::code_workspace::CodeTerminalOpenInput;
#[cfg(unix)]
use crate::commands::code_workspace::tests::{
    create_test_repository, method_count, persisted_local_binding, stateful_fake_codex,
};
#[cfg(unix)]
use std::sync::{Arc, Barrier};
#[cfg(unix)]
use tauri::ipc::Channel;

fn scope(project_dtag: &str) -> crate::code_workspace::CodeThreadBindingScope {
    crate::code_workspace::CodeThreadBindingScope {
        community_id: "community-1".to_string(),
        project_dtag: project_dtag.to_string(),
        repository_identity: "a".repeat(64),
    }
}

fn persisted_binding(
    app_data: &Path,
    execution_root: &Path,
) -> Result<CodeThreadBindingStore, String> {
    let execution_root = execution_root
        .canonicalize()
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .into_owned();
    let store = CodeThreadBindingStore::for_app_data(app_data)?;
    store.upsert(CodeThreadBinding {
        community_id: "community-1".to_string(),
        project_dtag: "project-1".to_string(),
        repository_identity: "a".repeat(64),
        codex_thread_id: "thread-1".to_string(),
        execution_mode: CodeExecutionMode::Local,
        execution_root,
        base_ref: "b".repeat(40),
        worktree_id: None,
    })?;
    Ok(store)
}

#[test]
fn wrong_scope_fails_before_runtime_or_lifecycle_side_effects() -> Result<(), String> {
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let root = tempfile::tempdir().map_err(|error| error.to_string())?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = persisted_binding(app_data.path(), root.path())?;
    let input = CodeThreadLifecycleInput {
        scope: scope("other-project"),
        thread_id: "thread-1".to_string(),
    };
    let result = archive_thread_native(
        input,
        app_data.path(),
        nest.path(),
        &crate::code_workspace::CodeRuntime::new(),
        &crate::code_workspace::CodeTerminalManager::new(),
        &Mutex::new(()),
        &AtomicBool::new(true),
    );
    assert!(result.is_err());
    let lookup = CodeThreadBindingLookupInput {
        scope: scope("project-1"),
        codex_thread_id: "thread-1".to_string(),
    };
    assert_eq!(
        store
            .lookup_with_lifecycle(&lookup)?
            .ok_or_else(|| "test binding disappeared".to_string())?
            .status,
        CodeThreadLifecycleStatus::Active
    );
    Ok(())
}

#[test]
fn unavailable_graph_marks_a_stable_target_unknown() -> Result<(), String> {
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let root = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = persisted_binding(app_data.path(), root.path())?;
    let lookup = lifecycle_lookup(&CodeThreadLifecycleInput {
        scope: scope("project-1"),
        thread_id: "thread-1".to_string(),
    });
    let lifecycle_authority = AtomicBool::new(true);
    let result = authoritative_graph_proof_or_mark_unknown(
        &store,
        &lookup,
        &crate::code_workspace::CodeRuntime::new(),
        &lifecycle_authority,
    );
    assert!(result.is_err());
    assert!(!lifecycle_authority.load(Ordering::Acquire));
    assert_eq!(
        store
            .lookup_with_lifecycle(&lookup)?
            .ok_or_else(|| "test binding disappeared".to_string())?
            .status,
        CodeThreadLifecycleStatus::Unknown
    );
    Ok(())
}

#[test]
fn empty_store_reconciliation_establishes_authority_without_runtime_rpc() -> Result<(), String> {
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    let lifecycle_authority = AtomicBool::new(false);

    let warning = reconcile_all_thread_lifecycles(
        &store,
        &crate::code_workspace::CodeRuntime::new(),
        &lifecycle_authority,
    )?;

    assert!(warning.is_none());
    assert!(lifecycle_authority.load(Ordering::Acquire));
    Ok(())
}

#[test]
fn definitely_unsent_delivery_restores_exact_stable_snapshot() -> Result<(), String> {
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let root = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = persisted_binding(app_data.path(), root.path())?;
    let lookup = CodeThreadBindingLookupInput {
        scope: scope("project-1"),
        codex_thread_id: "thread-1".to_string(),
    };
    let claim = store.begin_archive(&lookup)?;

    let result = handle_delivery_error(
        &store,
        &claim,
        "archive",
        CodeRpcDeliveryError::NotSent("synthetic pre-write refusal".to_string()),
    );

    assert!(result.is_err());
    assert_eq!(
        store
            .lookup_with_lifecycle(&lookup)?
            .ok_or_else(|| "test binding disappeared".to_string())?
            .status,
        CodeThreadLifecycleStatus::Active
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn archive_drains_terminal_before_rpc_and_unarchive_reconciles_notifications() -> Result<(), String>
{
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let repository = create_test_repository()?;
    let (scope, binding) = persisted_local_binding(&repository, "thread-1")?;
    let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    store.upsert(binding)?;

    let source = crate::code_workspace::code_thread_source("67f11a1d-0274-4d40-9b0c-e406e51c64fb")?;
    let execution_root = store
        .lookup(&CodeThreadBindingLookupInput {
            scope: scope.clone(),
            codex_thread_id: "thread-1".to_string(),
        })?
        .ok_or_else(|| "test binding disappeared".to_string())?
        .execution_root;
    let fake = stateful_fake_codex(&execution_root, &source, "thread-1", false)?;
    fake.mark_created()?;
    let runtime = crate::code_workspace::CodeRuntime::with_executable(fake.executable.clone());
    runtime.start(std::sync::Arc::new(|_| {}))?;
    let lifecycle_authority = AtomicBool::new(false);
    assert!(reconcile_all_thread_lifecycles(&store, &runtime, &lifecycle_authority,)?.is_none());

    let terminal_manager = crate::code_workspace::CodeTerminalManager::new();
    terminal_manager.install_test_owner(&scope, "thread-1", fake.terminal_drained_marker())?;
    let input = CodeThreadLifecycleInput {
        scope: scope.clone(),
        thread_id: "thread-1".to_string(),
    };
    let binding_lock = Mutex::new(());
    let archived = archive_thread_native(
        input.clone(),
        app_data.path(),
        nest.path(),
        &runtime,
        &terminal_manager,
        &binding_lock,
        &lifecycle_authority,
    )?;
    assert_eq!(archived.lifecycle, CodeThreadLifecycleStatus::Archived);
    assert!(archived.thread.is_none());
    assert!(fake.terminal_drained_marker().is_file());
    assert!(!runtime.is_thread_lifecycle_dirty("thread-1")?);

    let unarchived = unarchive_thread_native(
        input,
        app_data.path(),
        nest.path(),
        &runtime,
        &binding_lock,
        &lifecycle_authority,
    )?;
    assert_eq!(unarchived.lifecycle, CodeThreadLifecycleStatus::Active);
    assert_eq!(
        unarchived.thread.as_ref().map(|thread| thread.id.as_str()),
        Some("thread-1")
    );
    assert!(!runtime.is_thread_lifecycle_dirty("thread-1")?);
    let requests = fake.recorded_requests()?;
    assert_eq!(method_count(&requests, "thread/archive"), 1);
    assert_eq!(method_count(&requests, "thread/unarchive"), 1);
    runtime.stop()?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn descendant_created_by_the_drained_activity_blocks_archive_rpc() -> Result<(), String> {
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let repository = create_test_repository()?;
    let (scope, binding) = persisted_local_binding(&repository, "thread-1")?;
    let execution_root = binding.execution_root.clone();
    let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    store.upsert(binding)?;
    let source = crate::code_workspace::code_thread_source("67f11a1d-0274-4d40-9b0c-e406e51c64fb")?;
    let fake = stateful_fake_codex(&execution_root, &source, "thread-1", false)?;
    fake.mark_created()?;
    fake.spawn_descendant_after_terminal_drain()?;
    let runtime = crate::code_workspace::CodeRuntime::with_executable(fake.executable.clone());
    runtime.start(std::sync::Arc::new(|_| {}))?;
    let lifecycle_authority = AtomicBool::new(false);
    reconcile_all_thread_lifecycles(&store, &runtime, &lifecycle_authority)?;
    let terminal_manager = crate::code_workspace::CodeTerminalManager::new();
    terminal_manager.install_test_owner(&scope, "thread-1", fake.terminal_drained_marker())?;

    let result = archive_thread_native(
        CodeThreadLifecycleInput {
            scope: scope.clone(),
            thread_id: "thread-1".to_string(),
        },
        app_data.path(),
        nest.path(),
        &runtime,
        &terminal_manager,
        &Mutex::new(()),
        &lifecycle_authority,
    );

    assert!(result.is_err());
    assert!(fake.terminal_drained_marker().is_file());
    let requests = fake.recorded_requests()?;
    assert_eq!(method_count(&requests, "thread/archive"), 0);
    let lookup = CodeThreadBindingLookupInput {
        scope,
        codex_thread_id: "thread-1".to_string(),
    };
    assert_eq!(
        store
            .lookup_with_lifecycle(&lookup)?
            .ok_or_else(|| "test binding disappeared".to_string())?
            .status,
        CodeThreadLifecycleStatus::Active
    );
    runtime.stop()?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn terminal_open_and_archive_share_one_binding_lifecycle_barrier() -> Result<(), String> {
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let repository = create_test_repository()?;
    let (scope, binding) = persisted_local_binding(&repository, "thread-1")?;
    let execution_root = binding.execution_root.clone();
    let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    store.upsert(binding)?;
    let source = crate::code_workspace::code_thread_source("67f11a1d-0274-4d40-9b0c-e406e51c64fb")?;
    let fake = stateful_fake_codex(&execution_root, &source, "thread-1", false)?;
    fake.mark_created()?;
    std::fs::write(fake.terminal_drained_marker(), b"ready").map_err(|error| error.to_string())?;
    let runtime = crate::code_workspace::CodeRuntime::with_executable(fake.executable.clone());
    runtime.start(std::sync::Arc::new(|_| {}))?;
    let lifecycle_authority = Arc::new(AtomicBool::new(false));
    reconcile_all_thread_lifecycles(&store, &runtime, &lifecycle_authority)?;

    let terminal_manager = crate::code_workspace::CodeTerminalManager::new();
    let binding_lock = Arc::new(Mutex::new(()));
    let held = binding_lock
        .lock()
        .map_err(|_| "test binding lock is unavailable".to_string())?;
    let start = Arc::new(Barrier::new(3));

    let open_thread = {
        let start = Arc::clone(&start);
        let binding_lock = Arc::clone(&binding_lock);
        let lifecycle_authority = Arc::clone(&lifecycle_authority);
        let runtime = runtime.clone();
        let terminal_manager = terminal_manager.clone();
        let scope = scope.clone();
        let app_data_dir = app_data.path().to_path_buf();
        let nest_root = nest.path().to_path_buf();
        std::thread::spawn(move || {
            start.wait();
            crate::commands::code_terminal::open_terminal_for_test(
                CodeTerminalOpenInput {
                    scope,
                    thread_id: "thread-1".to_string(),
                    cols: 80,
                    rows: 24,
                },
                Channel::new(|_| Ok(())),
                &app_data_dir,
                &nest_root,
                (
                    &runtime,
                    &terminal_manager,
                    &binding_lock,
                    &lifecycle_authority,
                ),
            )
        })
    };
    let archive_thread = {
        let start = Arc::clone(&start);
        let binding_lock = Arc::clone(&binding_lock);
        let lifecycle_authority = Arc::clone(&lifecycle_authority);
        let runtime = runtime.clone();
        let terminal_manager = terminal_manager.clone();
        let scope = scope.clone();
        let app_data_dir = app_data.path().to_path_buf();
        let nest_root = nest.path().to_path_buf();
        std::thread::spawn(move || {
            start.wait();
            archive_thread_native(
                CodeThreadLifecycleInput {
                    scope,
                    thread_id: "thread-1".to_string(),
                },
                &app_data_dir,
                &nest_root,
                &runtime,
                &terminal_manager,
                &binding_lock,
                &lifecycle_authority,
            )
        })
    };

    start.wait();
    std::thread::sleep(std::time::Duration::from_millis(20));
    drop(held);
    let open_result = open_thread
        .join()
        .map_err(|_| "terminal open test thread panicked".to_string())?;
    let archived = archive_thread
        .join()
        .map_err(|_| "archive test thread panicked".to_string())??;

    assert_eq!(archived.lifecycle, CodeThreadLifecycleStatus::Archived);
    if let Err(error) = open_result {
        assert!(error.contains("not executable"));
    }
    assert_eq!(
        method_count(&fake.recorded_requests()?, "thread/archive"),
        1
    );
    terminal_manager.shutdown()?;
    runtime.stop()?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn active_turn_response_blocks_archive_before_rpc() -> Result<(), String> {
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let repository = create_test_repository()?;
    let (scope, binding) = persisted_local_binding(&repository, "thread-1")?;
    let execution_root = binding.execution_root.clone();
    let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    store.upsert(binding)?;
    let source = crate::code_workspace::code_thread_source("67f11a1d-0274-4d40-9b0c-e406e51c64fb")?;
    let fake = stateful_fake_codex(&execution_root, &source, "thread-1", false)?;
    fake.mark_created()?;
    let runtime = crate::code_workspace::CodeRuntime::with_executable(fake.executable.clone());
    runtime.start(std::sync::Arc::new(|_| {}))?;
    let lifecycle_authority = AtomicBool::new(false);
    reconcile_all_thread_lifecycles(&store, &runtime, &lifecycle_authority)?;
    runtime.turn_start_at(
        crate::code_workspace::CodeTurnStartInput {
            scope: scope.clone(),
            thread_id: "thread-1".to_string(),
            prompt: "Keep this turn active".to_string(),
            model: None,
            effort: None,
        },
        &execution_root,
    )?;

    let result = archive_thread_native(
        CodeThreadLifecycleInput {
            scope: scope.clone(),
            thread_id: "thread-1".to_string(),
        },
        app_data.path(),
        nest.path(),
        &runtime,
        &crate::code_workspace::CodeTerminalManager::new(),
        &Mutex::new(()),
        &lifecycle_authority,
    );
    assert!(result.is_err());
    let requests = fake.recorded_requests()?;
    assert_eq!(method_count(&requests, "turn/start"), 1);
    assert_eq!(method_count(&requests, "thread/archive"), 0);
    let lookup = CodeThreadBindingLookupInput {
        scope,
        codex_thread_id: "thread-1".to_string(),
    };
    assert_eq!(
        store
            .lookup_with_lifecycle(&lookup)?
            .ok_or_else(|| "test binding disappeared".to_string())?
            .status,
        CodeThreadLifecycleStatus::Active
    );
    runtime.stop()?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn pending_approval_blocks_archive_before_rpc_and_preserves_active_lifecycle() -> Result<(), String>
{
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let repository = create_test_repository()?;
    let (scope, binding) = persisted_local_binding(&repository, "thread-1")?;
    let execution_root = binding.execution_root.clone();
    let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    store.upsert(binding)?;
    let source = crate::code_workspace::code_thread_source("67f11a1d-0274-4d40-9b0c-e406e51c64fb")?;
    let fake = stateful_fake_codex(&execution_root, &source, "thread-1", false)?;
    fake.mark_created()?;
    let runtime = crate::code_workspace::CodeRuntime::with_executable(fake.executable.clone());
    runtime.start(std::sync::Arc::new(|_| {}))?;
    let lifecycle_authority = AtomicBool::new(false);
    reconcile_all_thread_lifecycles(&store, &runtime, &lifecycle_authority)?;
    fake.request_approval_on_read()?;

    let result = archive_thread_native(
        CodeThreadLifecycleInput {
            scope: scope.clone(),
            thread_id: "thread-1".to_string(),
        },
        app_data.path(),
        nest.path(),
        &runtime,
        &crate::code_workspace::CodeTerminalManager::new(),
        &Mutex::new(()),
        &lifecycle_authority,
    );
    assert!(result.is_err());
    assert!(runtime.has_pending_approval("thread-1")?);
    let requests = fake.recorded_requests()?;
    assert_eq!(method_count(&requests, "thread/archive"), 0);
    let lookup = CodeThreadBindingLookupInput {
        scope,
        codex_thread_id: "thread-1".to_string(),
    };
    assert_eq!(
        store
            .lookup_with_lifecycle(&lookup)?
            .ok_or_else(|| "test binding disappeared".to_string())?
            .status,
        CodeThreadLifecycleStatus::Active
    );
    runtime.stop()?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn uncertain_archive_response_persists_unknown_without_retrying_rpc() -> Result<(), String> {
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let repository = create_test_repository()?;
    let (scope, binding) = persisted_local_binding(&repository, "thread-1")?;
    let execution_root = binding.execution_root.clone();
    let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    store.upsert(binding)?;
    let source = crate::code_workspace::code_thread_source("67f11a1d-0274-4d40-9b0c-e406e51c64fb")?;
    let fake = stateful_fake_codex(&execution_root, &source, "thread-1", false)?;
    fake.mark_created()?;
    fake.fail_archive_response()?;
    let runtime = crate::code_workspace::CodeRuntime::with_executable(fake.executable.clone());
    runtime.start(std::sync::Arc::new(|_| {}))?;
    let lifecycle_authority = AtomicBool::new(false);
    reconcile_all_thread_lifecycles(&store, &runtime, &lifecycle_authority)?;
    let terminal_manager = crate::code_workspace::CodeTerminalManager::new();
    terminal_manager.install_test_owner(&scope, "thread-1", fake.terminal_drained_marker())?;

    let result = archive_thread_native(
        CodeThreadLifecycleInput {
            scope: scope.clone(),
            thread_id: "thread-1".to_string(),
        },
        app_data.path(),
        nest.path(),
        &runtime,
        &terminal_manager,
        &Mutex::new(()),
        &lifecycle_authority,
    );
    assert!(result.is_err());
    let requests = fake.recorded_requests()?;
    assert_eq!(method_count(&requests, "thread/archive"), 1);
    let lookup = CodeThreadBindingLookupInput {
        scope,
        codex_thread_id: "thread-1".to_string(),
    };
    assert_eq!(
        store
            .lookup_with_lifecycle(&lookup)?
            .ok_or_else(|| "test binding disappeared".to_string())?
            .status,
        CodeThreadLifecycleStatus::Unknown
    );
    runtime.stop()?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn archive_commit_failure_leaves_durable_transition_and_dirty_gate() -> Result<(), String> {
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let repository = create_test_repository()?;
    let (scope, binding) = persisted_local_binding(&repository, "thread-1")?;
    let execution_root = binding.execution_root.clone();
    let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    store.upsert(binding)?;
    let source = crate::code_workspace::code_thread_source("67f11a1d-0274-4d40-9b0c-e406e51c64fb")?;
    let fake = stateful_fake_codex(&execution_root, &source, "thread-1", false)?;
    fake.mark_created()?;
    let code_dir = app_data.path().join("code");
    let original_permissions = std::fs::metadata(&code_dir)
        .map_err(|error| error.to_string())?
        .permissions();
    fake.fail_archive_commit(&code_dir)?;
    let runtime = crate::code_workspace::CodeRuntime::with_executable(fake.executable.clone());
    runtime.start(std::sync::Arc::new(|_| {}))?;
    let lifecycle_authority = AtomicBool::new(false);
    reconcile_all_thread_lifecycles(&store, &runtime, &lifecycle_authority)?;
    let terminal_manager = crate::code_workspace::CodeTerminalManager::new();
    terminal_manager.install_test_owner(&scope, "thread-1", fake.terminal_drained_marker())?;

    let result = archive_thread_native(
        CodeThreadLifecycleInput {
            scope: scope.clone(),
            thread_id: "thread-1".to_string(),
        },
        app_data.path(),
        nest.path(),
        &runtime,
        &terminal_manager,
        &Mutex::new(()),
        &lifecycle_authority,
    );
    std::fs::set_permissions(&code_dir, original_permissions).map_err(|error| error.to_string())?;
    assert!(result.is_err());
    let requests = fake.recorded_requests()?;
    assert_eq!(method_count(&requests, "thread/archive"), 1);
    let lookup = CodeThreadBindingLookupInput {
        scope,
        codex_thread_id: "thread-1".to_string(),
    };
    assert_eq!(
        store
            .lookup_with_lifecycle(&lookup)?
            .ok_or_else(|| "test binding disappeared".to_string())?
            .status,
        CodeThreadLifecycleStatus::Archiving
    );
    assert!(!lifecycle_authority.load(Ordering::Acquire));
    assert!(runtime.is_thread_lifecycle_dirty("thread-1")?);
    runtime.stop()?;
    Ok(())
}
