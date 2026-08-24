use super::*;

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
    crate::commands::code_thread_lifecycle::invalidate_lifecycle_authority(&lifecycle_authority);

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
