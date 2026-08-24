use super::*;

#[test]
fn lifecycle_notifications_and_runtime_boundaries_keep_dirty_gate_fail_closed() -> Result<(), String>
{
    let bridge = EventBridge::new();
    bridge.reset(7, noop_emitter())?;
    let initial = bridge.lifecycle_dirty_checkpoint(7, "thread-1")?;
    assert!(initial.is_dirty());
    assert!(!initial.accepts_archive_completion());
    assert!(!initial.accepts_unarchive_completion());
    bridge.clear_lifecycle_dirty(7, "thread-1", initial)?;
    assert!(!bridge.lifecycle_dirty_checkpoint(7, "thread-1")?.is_dirty());

    let clean = bridge.lifecycle_dirty_checkpoint(7, "thread-1")?;
    assert!(clean.accepts_archive_completion());
    assert!(clean.accepts_unarchive_completion());
    bridge.publish(
        7,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-1".to_string()),
            turn_id: None,
            item_id: None,
            kind: "thread/archived".to_string(),
            payload: json!({ "threadId": "thread-1" }),
        },
    );
    assert!(bridge.lifecycle_dirty_checkpoint(7, "thread-1")?.is_dirty());
    assert!(bridge.clear_lifecycle_dirty(7, "thread-1", clean).is_err());
    let archived = bridge.lifecycle_dirty_checkpoint(7, "thread-1")?;
    assert!(archived.accepts_archive_completion());
    assert!(!archived.accepts_unarchive_completion());
    bridge.clear_lifecycle_dirty(7, "thread-1", archived.clone())?;
    assert!(!bridge.lifecycle_dirty_checkpoint(7, "thread-1")?.is_dirty());

    bridge.publish(
        7,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-1".to_string()),
            turn_id: None,
            item_id: None,
            kind: "thread/archived".to_string(),
            payload: json!({ "threadId": "thread-1" }),
        },
    );
    let archive_completion = bridge.lifecycle_dirty_checkpoint(7, "thread-1")?;
    bridge.publish(
        7,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-1".to_string()),
            turn_id: None,
            item_id: None,
            kind: "thread/unarchived".to_string(),
            payload: json!({ "threadId": "thread-1" }),
        },
    );
    assert!(archive_completion.accepts_archive_completion());
    assert!(bridge
        .clear_lifecycle_dirty(7, "thread-1", archive_completion)
        .is_err());
    let unarchived = bridge.lifecycle_dirty_checkpoint(7, "thread-1")?;
    assert!(unarchived.is_dirty());
    assert!(!unarchived.accepts_archive_completion());
    assert!(unarchived.accepts_unarchive_completion());
    bridge.clear_lifecycle_dirty(7, "thread-1", unarchived)?;

    let before_foreign_change = bridge.lifecycle_dirty_checkpoint(7, "thread-1")?;
    bridge.publish(
        7,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-descendant".to_string()),
            turn_id: None,
            item_id: None,
            kind: "thread/archived".to_string(),
            payload: json!({ "threadId": "thread-descendant" }),
        },
    );
    assert!(bridge
        .clear_lifecycle_dirty(7, "thread-1", before_foreign_change)
        .is_err());
    let after_foreign_change = bridge.lifecycle_dirty_checkpoint(7, "thread-1")?;
    bridge.clear_lifecycle_dirty(7, "thread-1", after_foreign_change)?;

    let before_stop = bridge.lifecycle_dirty_checkpoint(7, "thread-1")?;
    bridge.clear_activity(7)?;
    assert!(bridge.lifecycle_dirty_checkpoint(7, "thread-1")?.is_dirty());
    assert!(bridge
        .clear_lifecycle_dirty(7, "thread-1", before_stop)
        .is_err());
    bridge.reset(8, noop_emitter())?;
    assert!(bridge.lifecycle_dirty_checkpoint(8, "thread-1")?.is_dirty());
    assert!(bridge
        .clear_lifecycle_dirty(8, "thread-1", archived)
        .is_err());
    Ok(())
}
#[test]
fn new_thread_clean_seam_cannot_hide_prior_or_concurrent_lifecycle_notifications(
) -> Result<(), String> {
    let bridge = EventBridge::new();
    bridge.reset(7, noop_emitter())?;
    bridge.publish(
        7,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-prior".to_string()),
            turn_id: None,
            item_id: None,
            kind: "thread/archived".to_string(),
            payload: json!({ "threadId": "thread-prior" }),
        },
    );
    assert!(bridge
        .mark_new_thread_lifecycle_clean(7, "thread-prior")
        .is_err());
    assert!(bridge
        .lifecycle_dirty_checkpoint(7, "thread-prior")?
        .is_dirty());

    bridge.mark_new_thread_lifecycle_clean(7, "thread-later")?;
    assert!(!bridge
        .lifecycle_dirty_checkpoint(7, "thread-later")?
        .is_dirty());
    bridge.publish(
        7,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-later".to_string()),
            turn_id: None,
            item_id: None,
            kind: "thread/unarchived".to_string(),
            payload: json!({ "threadId": "thread-later" }),
        },
    );
    assert!(bridge
        .lifecycle_dirty_checkpoint(7, "thread-later")?
        .is_dirty());
    assert!(bridge
        .mark_new_thread_lifecycle_clean(7, "thread-later")
        .is_err());

    bridge.clear_activity(7)?;
    assert!(bridge
        .mark_new_thread_lifecycle_clean(7, "thread-after-boundary")
        .is_err());
    Ok(())
}
#[test]
fn thread_started_invalidates_graph_epoch_without_dirtying_new_thread_lifecycle(
) -> Result<(), String> {
    let bridge = EventBridge::new();
    bridge.reset(7, noop_emitter())?;
    let (_, before) = bridge.topology_checkpoint(7)?;

    bridge.publish(
        7,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-new".to_string()),
            turn_id: None,
            item_id: None,
            kind: "thread/started".to_string(),
            payload: json!({ "thread": { "id": "thread-new" } }),
        },
    );

    let (_, after) = bridge.topology_checkpoint(7)?;
    assert!(after > before);
    bridge.mark_new_thread_lifecycle_clean(7, "thread-new")?;
    assert!(!bridge
        .lifecycle_dirty_checkpoint(7, "thread-new")?
        .is_dirty());
    Ok(())
}

#[test]
fn topology_epoch_rejects_descendant_started_between_membership_scans() -> Result<(), String> {
    let bridge = EventBridge::new();
    bridge.reset(7, noop_emitter())?;
    let (boundary, revision) = bridge.topology_checkpoint(7)?;
    let deferred = HashSet::new();
    let graph = collect_authoritative_thread_graph(&deferred, |method, params| match method {
        "thread/list" if params["archived"] == false => Ok(json!({
            "data": [{
                "id": "thread-parent",
                "cwd": "/tmp/schoolx-code",
                "source": "appServer",
                "status": { "type": "idle" },
                "parentThreadId": null,
                "forkedFromId": null
            }],
            "nextCursor": null
        })),
        "thread/list" if params["archived"] == true => {
            bridge.publish(
                7,
                CodeWorkspaceEventDraft {
                    thread_id: Some("thread-child".to_string()),
                    turn_id: None,
                    item_id: None,
                    kind: "thread/started".to_string(),
                    payload: json!({ "thread": { "id": "thread-child" } }),
                },
            );
            Ok(json!({ "data": [], "nextCursor": null }))
        }
        "thread/loaded/list" => Ok(json!({ "data": [], "nextCursor": null })),
        _ => Err(format!("unexpected authoritative method {method}")),
    })?;
    assert_eq!(
        graph.membership("thread-parent"),
        Some(CodeThreadMembership::Active)
    );
    assert!(bridge
        .confirm_topology_checkpoint(7, boundary, revision)
        .is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn guarded_archive_rejects_post_graph_topology_before_writing_rpc_bytes() -> Result<(), String> {
    let (_directory, executable) = fake_codex(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
IFS= read -r initialize
printf '%s\n' '{"id":1,"result":{"userAgent":"codex-test","codexHome":"/tmp/codex-home","platformFamily":"unix","platformOs":"macos"}}'
IFS= read -r initialized
: > "$0.requests"
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$0.requests"
done
"#,
    )?;
    let runtime = CodeRuntime::with_executable(executable.clone());
    let ready = runtime.start(noop_emitter())?;
    let request_log = executable.with_file_name("codex.requests");
    let deadline = Instant::now() + Duration::from_secs(1);
    while !request_log.exists() {
        if Instant::now() >= deadline {
            return Err("guarded archive test request log was not created".to_string());
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let recovery_root = tempfile::tempdir().map_err(|error| error.to_string())?;
    let recovery_checkpoint = runtime.thread_lifecycle_dirty_checkpoint("thread-recovery")?;
    runtime.events.publish(
        ready.generation,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-recovery".to_string()),
            turn_id: None,
            item_id: None,
            kind: "thread/archived".to_string(),
            payload: json!({ "threadId": "thread-recovery" }),
        },
    );
    assert!(runtime
        .thread_resume_recovery_at_guarded(
            CodeThreadResumeInput {
                scope: binding_scope(),
                thread_id: "thread-recovery".to_string(),
                model: None,
            },
            &recovery_root.path().to_string_lossy(),
            recovery_checkpoint,
        )
        .is_err());
    assert!(recorded_requests(&executable)?.is_empty());

    runtime.commit_new_thread_lifecycle("thread-target", || Ok(()))?;
    let (topology_boundary_revision, topology_revision) =
        runtime.events.topology_checkpoint(ready.generation)?;
    let graph = CodeAuthoritativeThreadGraph::from_threads([
        super::super::super::thread_lifecycle::CodeAuthoritativeThread {
            id: "thread-target".to_string(),
            membership: CodeThreadMembership::Active,
            cwd: "/tmp/schoolx-code".to_string(),
            parent_thread_id: None,
            forked_from_id: None,
            status: CodePinnedThreadStatus::Idle,
        },
    ])?;
    let proof = CodeThreadLifecycleGraphProof {
        generation: ready.generation,
        thread_id: "thread-target".to_string(),
        graph,
        topology_boundary_revision,
        topology_revision,
    };
    runtime.events.publish(
        ready.generation,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-child".to_string()),
            turn_id: None,
            item_id: None,
            kind: "thread/started".to_string(),
            payload: json!({ "thread": { "id": "thread-child" } }),
        },
    );

    let error = runtime
        .thread_archive_guarded(
            &CodeThreadLifecycleInput {
                scope: binding_scope(),
                thread_id: "thread-target".to_string(),
            },
            proof,
        )
        .err()
        .ok_or_else(|| "stale graph proof unexpectedly wrote archive RPC".to_string())?;
    assert!(error.definitely_not_sent());
    assert!(recorded_requests(&executable)?.is_empty());

    let (started_tx, started_rx) = mpsc::sync_channel(0);
    let (done_tx, done_rx) = mpsc::sync_channel(0);
    let mut publisher = None;
    let events = Arc::clone(&runtime.events);
    runtime.commit_new_thread_lifecycle("thread-new", || {
        publisher = Some(std::thread::spawn(move || {
            let _ = started_tx.send(());
            events.publish(
                ready.generation,
                CodeWorkspaceEventDraft {
                    thread_id: Some("thread-new".to_string()),
                    turn_id: None,
                    item_id: None,
                    kind: "thread/archived".to_string(),
                    payload: json!({ "threadId": "thread-new" }),
                },
            );
            let _ = done_tx.send(());
        }));
        started_rx.recv().map_err(|error| error.to_string())?;
        assert!(done_rx.recv_timeout(Duration::from_millis(20)).is_err());
        Ok(())
    })?;
    done_rx.recv().map_err(|error| error.to_string())?;
    publisher
        .ok_or_else(|| "new-thread publisher was not started".to_string())?
        .join()
        .map_err(|_| "new-thread publisher test thread panicked".to_string())?;
    assert!(runtime
        .thread_lifecycle_dirty_checkpoint("thread-new")?
        .is_dirty());
    runtime.stop()?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn fork_commit_revalidates_the_exact_source_after_response() -> Result<(), String> {
    let (_directory, executable) = fake_codex(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
IFS= read -r initialize
printf '%s\n' '{"id":1,"result":{"userAgent":"codex-test","codexHome":"/tmp/codex-home","platformFamily":"unix","platformOs":"macos"}}'
IFS= read -r initialized
while IFS= read -r line; do :; done
"#,
    )?;
    let runtime = CodeRuntime::with_executable(executable);
    let ready = runtime.start(noop_emitter())?;

    runtime.commit_new_thread_lifecycle("thread-source", || Ok(()))?;
    let checkpoint = runtime.thread_lifecycle_dirty_checkpoint("thread-source")?;
    let completion = CodeThreadForkCompletion {
        generation: ready.generation,
        source_thread_id: "thread-source".to_string(),
        lifecycle_checkpoint: checkpoint,
        activity_revision: 0,
    };
    runtime.events.publish(
        ready.generation,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-source".to_string()),
            turn_id: None,
            item_id: None,
            kind: "thread/archived".to_string(),
            payload: json!({ "threadId": "thread-source" }),
        },
    );
    let committed = AtomicBool::new(false);
    assert!(runtime
        .commit_new_fork_lifecycle("thread-source", "thread-child", completion, || {
            committed.store(true, Ordering::Release);
            Ok(())
        },)
        .is_err());
    assert!(!committed.load(Ordering::Acquire));
    assert!(runtime
        .thread_lifecycle_dirty_checkpoint("thread-child")?
        .is_dirty());

    runtime.commit_new_thread_lifecycle("thread-source-clean", || Ok(()))?;
    let clean_checkpoint = runtime.thread_lifecycle_dirty_checkpoint("thread-source-clean")?;
    let clean_completion = CodeThreadForkCompletion {
        generation: ready.generation,
        source_thread_id: "thread-source-clean".to_string(),
        lifecycle_checkpoint: clean_checkpoint,
        activity_revision: 0,
    };
    runtime.commit_new_fork_lifecycle(
        "thread-source-clean",
        "thread-child-clean",
        clean_completion,
        || Ok(()),
    )?;
    assert!(!runtime
        .thread_lifecycle_dirty_checkpoint("thread-child-clean")?
        .is_dirty());
    runtime.stop()?;
    Ok(())
}

#[test]
fn lifecycle_completion_atomically_commits_only_one_expected_signal() -> Result<(), String> {
    fn receipt(
        bridge: &EventBridge,
        expected: CodeThreadLifecycleSignal,
    ) -> Result<LifecycleWriteReceipt, String> {
        let checkpoint = bridge.lifecycle_dirty_checkpoint(7, "thread-target")?;
        let (topology_boundary_revision, topology_revision) = bridge.topology_checkpoint(7)?;
        Ok(LifecycleWriteReceipt {
            generation: 7,
            thread_id: "thread-target".to_string(),
            expected,
            lifecycle_boundary_revision: checkpoint.boundary_revision,
            topology_boundary_revision,
            topology_revision,
        })
    }

    let bridge = EventBridge::new();
    bridge.reset(7, noop_emitter())?;
    bridge.mark_new_thread_lifecycle_clean(7, "thread-target")?;

    let completion = bridge
        .mutation_response_checkpoint(receipt(&bridge, CodeThreadLifecycleSignal::Archived)?)?;
    bridge.publish(
        7,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-target".to_string()),
            turn_id: None,
            item_id: None,
            kind: "thread/archived".to_string(),
            payload: json!({ "threadId": "thread-target" }),
        },
    );
    let mut commit_calls = 0_usize;
    let committed = bridge.complete_lifecycle_mutation(
        "thread-target",
        completion,
        CodeThreadLifecycleSignal::Archived,
        || {
            commit_calls = commit_calls.saturating_add(1);
            Ok("saved")
        },
    )?;
    assert_eq!(committed, "saved");
    assert_eq!(commit_calls, 1);
    assert!(!bridge
        .lifecycle_dirty_checkpoint(7, "thread-target")?
        .is_dirty());

    let completion = bridge
        .mutation_response_checkpoint(receipt(&bridge, CodeThreadLifecycleSignal::Archived)?)?;
    bridge.publish(
        7,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-target".to_string()),
            turn_id: None,
            item_id: None,
            kind: "thread/unarchived".to_string(),
            payload: json!({ "threadId": "thread-target" }),
        },
    );
    let mut conflicting_commit_calls = 0_usize;
    assert!(bridge
        .complete_lifecycle_mutation(
            "thread-target",
            completion,
            CodeThreadLifecycleSignal::Archived,
            || {
                conflicting_commit_calls = conflicting_commit_calls.saturating_add(1);
                Ok(())
            },
        )
        .is_err());
    assert_eq!(conflicting_commit_calls, 0);

    bridge.clear_lifecycle_dirty(
        7,
        "thread-target",
        bridge.lifecycle_dirty_checkpoint(7, "thread-target")?,
    )?;
    let completion = bridge
        .mutation_response_checkpoint(receipt(&bridge, CodeThreadLifecycleSignal::Archived)?)?;
    for _ in 0..2 {
        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-target".to_string()),
                turn_id: None,
                item_id: None,
                kind: "thread/archived".to_string(),
                payload: json!({ "threadId": "thread-target" }),
            },
        );
    }
    let mut duplicate_commit_calls = 0_usize;
    assert!(bridge
        .complete_lifecycle_mutation(
            "thread-target",
            completion,
            CodeThreadLifecycleSignal::Archived,
            || {
                duplicate_commit_calls = duplicate_commit_calls.saturating_add(1);
                Ok(())
            },
        )
        .is_err());
    assert_eq!(duplicate_commit_calls, 0);
    Ok(())
}

#[test]
fn lifecycle_completion_rejects_foreign_topology_and_signal_reordering() -> Result<(), String> {
    fn clean_bridge() -> Result<EventBridge, String> {
        let bridge = EventBridge::new();
        bridge.reset(7, noop_emitter())?;
        bridge.mark_new_thread_lifecycle_clean(7, "thread-target")?;
        Ok(bridge)
    }

    let bridge = clean_bridge()?;
    let checkpoint = bridge.lifecycle_dirty_checkpoint(7, "thread-target")?;
    let (boundary, revision) = bridge.topology_checkpoint(7)?;
    let receipt = LifecycleWriteReceipt {
        generation: 7,
        thread_id: "thread-target".to_string(),
        expected: CodeThreadLifecycleSignal::Archived,
        lifecycle_boundary_revision: checkpoint.boundary_revision,
        topology_boundary_revision: boundary,
        topology_revision: revision,
    };
    for kind in ["thread/archived", "thread/unarchived", "thread/archived"] {
        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-target".to_string()),
                turn_id: None,
                item_id: None,
                kind: kind.to_string(),
                payload: json!({ "threadId": "thread-target" }),
            },
        );
    }
    assert!(bridge.mutation_response_checkpoint(receipt).is_err());

    let bridge = clean_bridge()?;
    let checkpoint = bridge.lifecycle_dirty_checkpoint(7, "thread-target")?;
    let (boundary, revision) = bridge.topology_checkpoint(7)?;
    let completion = bridge.mutation_response_checkpoint(LifecycleWriteReceipt {
        generation: 7,
        thread_id: "thread-target".to_string(),
        expected: CodeThreadLifecycleSignal::Archived,
        lifecycle_boundary_revision: checkpoint.boundary_revision,
        topology_boundary_revision: boundary,
        topology_revision: revision,
    })?;
    bridge.publish(
        7,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-foreign".to_string()),
            turn_id: None,
            item_id: None,
            kind: "thread/started".to_string(),
            payload: json!({ "thread": { "id": "thread-foreign" } }),
        },
    );
    let mut commit_calls = 0_usize;
    assert!(bridge
        .complete_lifecycle_mutation(
            "thread-target",
            completion,
            CodeThreadLifecycleSignal::Archived,
            || {
                commit_calls = commit_calls.saturating_add(1);
                Ok(())
            },
        )
        .is_err());
    assert_eq!(commit_calls, 0);
    Ok(())
}

#[test]
fn lifecycle_signal_waiting_on_durable_commit_barrier_remains_dirty_after_success(
) -> Result<(), String> {
    let bridge = Arc::new(EventBridge::new());
    bridge.reset(7, noop_emitter())?;
    bridge.mark_new_thread_lifecycle_clean(7, "thread-target")?;
    let checkpoint = bridge.lifecycle_dirty_checkpoint(7, "thread-target")?;
    let (topology_boundary_revision, topology_revision) = bridge.topology_checkpoint(7)?;
    let completion = bridge.mutation_response_checkpoint(LifecycleWriteReceipt {
        generation: 7,
        thread_id: "thread-target".to_string(),
        expected: CodeThreadLifecycleSignal::Archived,
        lifecycle_boundary_revision: checkpoint.boundary_revision,
        topology_boundary_revision,
        topology_revision,
    })?;

    let (started_tx, started_rx) = mpsc::sync_channel(0);
    let (done_tx, done_rx) = mpsc::sync_channel(0);
    let mut publisher = None;
    let publisher_bridge = Arc::clone(&bridge);
    bridge.complete_lifecycle_mutation(
        "thread-target",
        completion,
        CodeThreadLifecycleSignal::Archived,
        || {
            publisher = Some(std::thread::spawn(move || {
                let _ = started_tx.send(());
                publisher_bridge.publish(
                    7,
                    CodeWorkspaceEventDraft {
                        thread_id: Some("thread-target".to_string()),
                        turn_id: None,
                        item_id: None,
                        kind: "thread/archived".to_string(),
                        payload: json!({ "threadId": "thread-target" }),
                    },
                );
                let _ = done_tx.send(());
            }));
            started_rx.recv().map_err(|error| error.to_string())?;
            assert!(done_rx.recv_timeout(Duration::from_millis(20)).is_err());
            Ok(())
        },
    )?;
    done_rx.recv().map_err(|error| error.to_string())?;
    publisher
        .ok_or_else(|| "lifecycle publisher was not started".to_string())?
        .join()
        .map_err(|_| "lifecycle publisher test thread panicked".to_string())?;
    assert!(bridge
        .lifecycle_dirty_checkpoint(7, "thread-target")?
        .is_dirty());
    Ok(())
}

#[test]
fn lifecycle_checkpoint_is_bound_to_one_exact_thread() -> Result<(), String> {
    let bridge = EventBridge::new();
    bridge.reset(7, noop_emitter())?;
    bridge.mark_new_thread_lifecycle_clean(7, "thread-a")?;
    bridge.mark_new_thread_lifecycle_clean(7, "thread-b")?;
    let checkpoint = bridge.lifecycle_dirty_checkpoint(7, "thread-a")?;
    let inner = bridge.inner.lock().map_err(|error| error.to_string())?;
    assert!(
        validate_exact_lifecycle_checkpoint_locked(&inner, 7, "thread-b", &checkpoint,).is_err()
    );
    Ok(())
}
