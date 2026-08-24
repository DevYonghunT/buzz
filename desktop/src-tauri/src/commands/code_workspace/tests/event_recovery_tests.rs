use super::*;

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
    assert!(validate_thread_identity_and_root("thread-b", None, "thread-a", &expected,).is_err());
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
