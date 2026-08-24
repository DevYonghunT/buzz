use super::*;

#[test]
pub(super) fn startup_recovery_precedes_runtime_start_and_lifecycle_reconciliation(
) -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let sentinel = b"startup recovery replacement bytes\n".to_vec();
    let mut hook = InstallOriginalReplacementAtQuarantine {
        original: fixture.managed_root.clone(),
        sentinel: sentinel.clone(),
        tripped: false,
    };
    let injected = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut hook,
    )
    .expect_err("startup fixture must stop after installing a replacement");
    assert!(injected.contains("injected original-path replacement"));
    assert!(hook.tripped);

    let removing = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "startup fixture lost its removal journal".to_string())?;
    if !matches!(removing, CodeWorktreeRemovalRecord::Removing(_)) {
        return Err("startup fixture did not leave sticky Removing".to_string());
    }
    let before = snapshot_removing_retry_state(&fixture, &removing)?;
    let fake = crate::commands::stateful_fake_codex(
        &path_string(&fixture.managed_root, "replacement managed root")?,
        &crate::code_workspace::code_thread_source(PREPARATION_ID)?,
        THREAD_ID,
        false,
    )?;
    let started_marker = fake.started_marker();
    let runtime = crate::code_workspace::CodeRuntime::with_executable(fake.executable.clone());
    let binding_lock = Mutex::new(());
    let lifecycle_authority = AtomicBool::new(true);

    let error = crate::commands::start_code_runtime_after_removal_recovery(
        &fixture.app_data,
        &fixture.nest_root,
        &runtime,
        &binding_lock,
        &lifecycle_authority,
        |_| Arc::new(|_| {}),
    )
    .expect_err("sticky startup recovery must fail before Codex starts");
    assert!(
        error.contains("coordinates contain a replacement or ambiguous state"),
        "unexpected startup recovery rejection: {error}"
    );
    assert!(
        !started_marker.exists(),
        "Codex started before pending physical recovery completed"
    );
    assert!(
        !lifecycle_authority.load(Ordering::Acquire),
        "startup recovery failure left lifecycle authority enabled"
    );
    assert_eq!(
        fixture.store.lookup_worktree_removal(&fixture.lookup)?,
        Some(removing.clone()),
        "startup recovery changed the sticky Removing journal"
    );
    assert_eq!(
        snapshot_removing_retry_state(&fixture, &removing)?,
        before,
        "rejected startup recovery mutated protected state"
    );
    assert_eq!(
        fs::read(fixture.managed_root.join("sentinel.txt")).map_err(|error| error.to_string())?,
        sentinel,
        "startup recovery changed the replacement root"
    );
    Ok(())
}

#[test]
pub(super) fn ready_runtime_start_skips_startup_only_removal_recovery() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let sentinel = b"ready runtime replacement bytes\n".to_vec();
    let mut hook = InstallOriginalReplacementAtQuarantine {
        original: fixture.managed_root.clone(),
        sentinel,
        tripped: false,
    };
    unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut hook,
    )
    .expect_err("ready-runtime fixture must stop after installing a replacement");
    if !hook.tripped {
        return Err("ready-runtime replacement hook did not run".to_string());
    }
    let removing = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "ready-runtime fixture lost its removal journal".to_string())?;
    let before = snapshot_removing_retry_state(&fixture, &removing)?;
    let fake = crate::commands::stateful_fake_codex(
        &path_string(&fixture.managed_root, "replacement managed root")?,
        &crate::code_workspace::code_thread_source(PREPARATION_ID)?,
        THREAD_ID,
        false,
    )?;
    let runtime = crate::code_workspace::CodeRuntime::with_executable(fake.executable.clone());
    let initial = runtime.start(Arc::new(|_| {}))?;
    let binding_lock = Mutex::new(());
    let lifecycle_authority = AtomicBool::new(true);

    let repeated = crate::commands::start_code_runtime_after_removal_recovery(
        &fixture.app_data,
        &fixture.nest_root,
        &runtime,
        &binding_lock,
        &lifecycle_authority,
        |_| Arc::new(|_| {}),
    )?;
    assert_eq!(repeated.generation, initial.generation);
    assert!(lifecycle_authority.load(Ordering::Acquire));
    assert_eq!(
        fixture.store.lookup_worktree_removal(&fixture.lookup)?,
        Some(removing.clone())
    );
    assert_eq!(snapshot_removing_retry_state(&fixture, &removing)?, before);
    runtime.stop()?;
    Ok(())
}

#[test]
pub(super) fn public_scope_thread_removal_returns_only_the_native_derived_receipt(
) -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let fake = crate::commands::stateful_fake_codex(
        &path_string(&fixture.managed_root, "managed root")?,
        &crate::code_workspace::code_thread_source(PREPARATION_ID)?,
        THREAD_ID,
        false,
    )?;
    let runtime = crate::code_workspace::CodeRuntime::with_executable(fake.executable.clone());
    runtime.start(Arc::new(|_| {}))?;
    let terminals = crate::code_workspace::CodeTerminalManager::new();
    let binding_lock = Mutex::new(());
    let lifecycle_authority = AtomicBool::new(true);
    let receipt = super::super::super::remove_archived_worktree(
        &fixture.store,
        binding_lock
            .lock()
            .map_err(|_| "test binding lock is unavailable".to_string())?,
        super::super::super::CodeWorktreeRemoveInput {
            scope: fixture.lookup.scope.clone(),
            thread_id: THREAD_ID.to_string(),
        },
        &fixture.nest_root,
        super::super::super::CodeWorktreeRemovalContext {
            runtime: &runtime,
            terminals: &terminals,
            lifecycle_authority_ready: &lifecycle_authority,
            shutdown_started: &AtomicBool::new(false),
        },
    )?;
    let removed = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "public removal did not persist its tombstone".to_string())?;
    assert_removed(&fixture, &removed)?;
    assert_eq!(receipt.removal_id, removed.authority().removal_id);
    assert_eq!(receipt.scope, fixture.lookup.scope);
    assert_eq!(receipt.thread_id, THREAD_ID);
    assert_eq!(
        receipt.worktree_id,
        removed.authority().merge_proof.worktree_id
    );
    assert_eq!(
        receipt.head_commit,
        removed.authority().merge_proof.head_commit
    );
    assert_eq!(
        receipt.merged_into_ref,
        removed.authority().merge_proof.target_ref
    );
    assert_eq!(
        receipt.merged_into_commit,
        removed.authority().merge_proof.target_commit
    );
    assert_eq!(
        receipt.transcript_disposition,
        super::super::super::CodeWorktreeTranscriptDisposition::Preserved
    );
    assert_eq!(
        receipt.execution_disposition,
        super::super::super::CodeWorktreeExecutionDisposition::Removed
    );
    runtime.stop()?;
    Ok(())
}

#[test]
pub(super) fn sealed_idle_no_pty_no_approval_clearance_serializes_concurrent_admission(
) -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let fake = crate::commands::stateful_fake_codex(
        &path_string(&fixture.managed_root, "managed root")?,
        &crate::code_workspace::code_thread_source(PREPARATION_ID)?,
        THREAD_ID,
        false,
    )?;
    let runtime = Arc::new(crate::code_workspace::CodeRuntime::with_executable(
        fake.executable.clone(),
    ));
    let generation = runtime.start(Arc::new(|_| {}))?.generation;
    let terminals = Arc::new(crate::code_workspace::CodeTerminalManager::new());
    let binding_lock = Arc::new(Mutex::new(()));
    let lifecycle_authority = Arc::new(AtomicBool::new(true));
    let binding_guard = binding_lock
        .lock()
        .map_err(|_| "test binding lock is unavailable".to_string())?;
    let clearance = prove_removal_activity_clearance(
        &runtime,
        &terminals,
        binding_guard,
        fixture.lookup.clone(),
    )?;
    let requests_before = fake.recorded_requests()?;
    let start = Arc::new(Barrier::new(4));

    let (turn_tx, turn_rx) = mpsc::sync_channel(1);
    let turn_thread = {
        let runtime = Arc::clone(&runtime);
        let start = Arc::clone(&start);
        let input = crate::code_workspace::CodeTurnStartInput {
            scope: fixture.lookup.scope.clone(),
            thread_id: THREAD_ID.to_string(),
            prompt: "sealed removal admission".to_string(),
            model: None,
            effort: None,
        };
        let managed_root = path_string(&fixture.managed_root, "managed root")?;
        std::thread::spawn(move || {
            start.wait();
            let _ = turn_tx.send(runtime.turn_start_at(input, &managed_root));
        })
    };

    let (approval_tx, approval_rx) = mpsc::sync_channel(1);
    let approval_thread = {
        let runtime = Arc::clone(&runtime);
        let start = Arc::clone(&start);
        std::thread::spawn(move || {
            start.wait();
            let _ = approval_tx.send(runtime.insert_pending_approval_for_test(
                generation,
                "sealed-removal-approval",
                THREAD_ID,
            ));
        })
    };

    let (terminal_tx, terminal_rx) = mpsc::sync_channel(1);
    let terminal_thread = {
        let runtime = Arc::clone(&runtime);
        let terminals = Arc::clone(&terminals);
        let binding_lock = Arc::clone(&binding_lock);
        let lifecycle_authority = Arc::clone(&lifecycle_authority);
        let start = Arc::clone(&start);
        let app_data = fixture.app_data.clone();
        let nest_root = fixture.nest_root.clone();
        let scope = fixture.lookup.scope.clone();
        std::thread::spawn(move || {
            start.wait();
            let result = crate::commands::open_terminal_for_test(
                crate::code_workspace::CodeTerminalOpenInput {
                    scope,
                    thread_id: THREAD_ID.to_string(),
                    cols: 80,
                    rows: 24,
                },
                tauri::ipc::Channel::new(|_| Ok(())),
                &app_data,
                &nest_root,
                (&runtime, &terminals, &binding_lock, &lifecycle_authority),
            );
            let _ = terminal_tx.send(result);
        })
    };

    start.wait();
    let blocked = Duration::from_millis(100);
    assert!(matches!(
        turn_rx.recv_timeout(blocked),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(matches!(
        approval_rx.recv_timeout(blocked),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(matches!(
        terminal_rx.recv_timeout(blocked),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert_eq!(
        fake.recorded_requests()?,
        requests_before,
        "turn bytes were admitted while removal clearance was held"
    );
    terminals.ensure_owner_absent(&fixture.lookup.scope, THREAD_ID)?;

    let removed = remove_archived_worktree_private(
        &fixture.store,
        clearance,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
    )?;
    if !matches!(removed, CodeWorktreeRemovalRecord::Removed(_)) {
        return Err("sealed removal did not finalize its tombstone".to_string());
    }
    assert_removed(&fixture, &removed)?;
    turn_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|error| format!("turn admission did not unblock: {error}"))??;
    approval_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|error| format!("approval admission did not unblock: {error}"))??;
    let terminal_error = terminal_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|error| format!("terminal admission did not unblock: {error}"))?
        .expect_err("a removed tombstone must not open a terminal after clearance");
    assert!(
        terminal_error.contains("is not bound to the requested SchoolX community"),
        "unexpected removed-terminal rejection: {terminal_error}"
    );
    turn_thread
        .join()
        .map_err(|_| "turn admission thread panicked".to_string())?;
    approval_thread
        .join()
        .map_err(|_| "approval admission thread panicked".to_string())?;
    terminal_thread
        .join()
        .map_err(|_| "terminal admission thread panicked".to_string())?;
    assert!(runtime.has_pending_approval(THREAD_ID)?);
    terminals.ensure_owner_absent(&fixture.lookup.scope, THREAD_ID)?;
    runtime.stop()?;
    terminals.shutdown()?;
    Ok(())
}
