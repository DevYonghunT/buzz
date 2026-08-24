use super::*;

#[test]
fn event_backlog_is_bounded_and_reports_a_replay_gap() -> Result<(), String> {
    let bridge = EventBridge::new();
    let approvals = PendingApprovalStore::default();
    bridge.reset(7, noop_emitter())?;
    approvals.reset(7);
    for index in 0..=MAX_NOTIFICATION_BACKLOG {
        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-1".to_string()),
                turn_id: None,
                item_id: None,
                kind: "warning".to_string(),
                payload: json!({ "index": index }),
            },
        );
    }

    let snapshot = bridge.snapshot(&approvals, Some(7), Some(0))?;
    assert_eq!(snapshot.events.len(), MAX_NOTIFICATION_BACKLOG);
    assert_eq!(snapshot.events.first().map(|event| event.sequence), Some(2));
    assert_eq!(snapshot.latest_sequence, 513);
    assert!(snapshot.truncated);
    let checkpoint = snapshot
        .checkpoint
        .ok_or_else(|| "truncated replay had no authoritative checkpoint".to_string())?;
    assert_eq!(checkpoint.runtime_generation, 7);
    assert_eq!(checkpoint.sequence_watermark, 513);
    assert!(checkpoint.active_turns.is_empty());
    assert!(checkpoint.pending_approvals.is_empty());
    Ok(())
}

#[test]
fn checkpoint_preserves_active_turn_and_pending_approval_after_eviction() -> Result<(), String> {
    let bridge = EventBridge::new();
    let approvals = PendingApprovalStore::default();
    bridge.reset(7, noop_emitter())?;
    approvals.reset(7);
    bridge.publish(
        7,
        CodeWorkspaceEventDraft {
            thread_id: Some("thread-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            item_id: None,
            kind: "turn/started".to_string(),
            payload: json!({ "turn": { "status": "inProgress" } }),
        },
    );
    let approval = approvals
        .insert_request(
            7,
            json!("approval-1"),
            "item/fileChange/requestApproval",
            Some(json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "item-1",
                "availableDecisions": ["accept", "decline"]
            })),
        )?
        .ok_or_else(|| "approval request was not normalized".to_string())?;
    bridge.publish(7, approval);
    for index in 0..MAX_NOTIFICATION_BACKLOG {
        bridge.publish(
            7,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-1".to_string()),
                turn_id: None,
                item_id: None,
                kind: "warning".to_string(),
                payload: json!({ "index": index }),
            },
        );
    }

    let snapshot = bridge.snapshot(&approvals, Some(7), Some(0))?;
    assert!(snapshot.truncated);
    assert_eq!(snapshot.events.first().map(|event| event.sequence), Some(3));
    let checkpoint = snapshot
        .checkpoint
        .ok_or_else(|| "evicted replay had no authoritative checkpoint".to_string())?;
    assert_eq!(checkpoint.sequence_watermark, 514);
    assert_eq!(checkpoint.active_turns.len(), 1);
    assert_eq!(checkpoint.active_turns[0].thread_id, "thread-1");
    assert_eq!(checkpoint.active_turns[0].turn_id, "turn-1");
    assert_eq!(checkpoint.active_turns[0].started_sequence, 1);
    assert_eq!(checkpoint.pending_approvals.len(), 1);
    assert!(checkpoint.pending_approvals[0].respondable);
    assert_eq!(checkpoint.pending_approvals[0].event.sequence, 514);
    assert_eq!(
        checkpoint.pending_approvals[0].event.payload["requestId"],
        "approval-1"
    );
    Ok(())
}

#[test]
fn approval_insert_resolve_and_turn_clear_share_the_event_admission_barrier() -> Result<(), String>
{
    let bridge = Arc::new(EventBridge::new());
    let approvals = Arc::new(PendingApprovalStore::default());
    bridge.reset(7, noop_emitter())?;
    approvals.reset(7);

    let event_guard = bridge.inner.lock().map_err(|error| error.to_string())?;
    let (started_tx, started_rx) = mpsc::sync_channel(0);
    let (done_tx, done_rx) = mpsc::sync_channel(0);
    let insertion_bridge = Arc::clone(&bridge);
    let insertion_approvals = Arc::clone(&approvals);
    let insertion = std::thread::spawn(move || {
        let _ = started_tx.send(());
        let result = insertion_bridge.insert_approval_and_publish(
            &insertion_approvals,
            7,
            json!("approval-1"),
            "item/fileChange/requestApproval",
            Some(json!({
                "threadId": "thread-1",
                "turnId": "turn-1",
                "itemId": "item-1"
            })),
        );
        let _ = done_tx.send(result);
    });
    started_rx.recv().map_err(|error| error.to_string())?;
    assert!(done_rx.recv_timeout(Duration::from_millis(20)).is_err());
    assert_eq!(approvals.len(), 0);
    drop(event_guard);
    assert!(done_rx.recv().map_err(|error| error.to_string())??);
    insertion
        .join()
        .map_err(|_| "approval insertion test thread panicked".to_string())?;
    assert_eq!(approvals.len(), 1);

    let event_guard = bridge.inner.lock().map_err(|error| error.to_string())?;
    let (started_tx, started_rx) = mpsc::sync_channel(0);
    let (done_tx, done_rx) = mpsc::sync_channel(0);
    let resolve_bridge = Arc::clone(&bridge);
    let resolve_approvals = Arc::clone(&approvals);
    let resolve = std::thread::spawn(move || {
        let _ = started_tx.send(());
        let raw = json!({
            "requestId": "approval-1",
            "threadId": "thread-1"
        });
        let result = resolve_bridge.publish_notification(
            &resolve_approvals,
            7,
            "serverRequest/resolved",
            Some(&raw),
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-1".to_string()),
                turn_id: Some("turn-1".to_string()),
                item_id: Some("item-1".to_string()),
                kind: "serverRequest/resolved".to_string(),
                payload: raw.clone(),
            },
        );
        let _ = done_tx.send(result);
    });
    started_rx.recv().map_err(|error| error.to_string())?;
    assert!(done_rx.recv_timeout(Duration::from_millis(20)).is_err());
    assert_eq!(approvals.len(), 1);
    drop(event_guard);
    done_rx.recv().map_err(|error| error.to_string())??;
    resolve
        .join()
        .map_err(|_| "approval resolve test thread panicked".to_string())?;
    assert_eq!(approvals.len(), 0);

    approvals.insert_request(
        7,
        json!("approval-2"),
        "item/fileChange/requestApproval",
        Some(json!({
            "threadId": "thread-1",
            "turnId": "turn-2",
            "itemId": "item-2"
        })),
    )?;
    let event_guard = bridge.inner.lock().map_err(|error| error.to_string())?;
    let (started_tx, started_rx) = mpsc::sync_channel(0);
    let (done_tx, done_rx) = mpsc::sync_channel(0);
    let completion_bridge = Arc::clone(&bridge);
    let completion_approvals = Arc::clone(&approvals);
    let completion = std::thread::spawn(move || {
        let _ = started_tx.send(());
        let result = completion_bridge.publish_notification(
            &completion_approvals,
            7,
            "turn/completed",
            None,
            CodeWorkspaceEventDraft {
                thread_id: Some("thread-1".to_string()),
                turn_id: Some("turn-2".to_string()),
                item_id: None,
                kind: "turn/completed".to_string(),
                payload: json!({ "turn": { "status": "completed" } }),
            },
        );
        let _ = done_tx.send(result);
    });
    started_rx.recv().map_err(|error| error.to_string())?;
    assert!(done_rx.recv_timeout(Duration::from_millis(20)).is_err());
    assert_eq!(approvals.len(), 1);
    drop(event_guard);
    done_rx.recv().map_err(|error| error.to_string())??;
    completion
        .join()
        .map_err(|_| "turn completion test thread panicked".to_string())?;
    assert_eq!(approvals.len(), 0);
    Ok(())
}

#[test]
fn failed_approval_write_restores_pending_request_for_retry() -> Result<(), String> {
    let approvals = PendingApprovalStore::default();
    approvals.reset(1);
    approvals.insert_request(
        1,
        json!("approval-1"),
        "item/fileChange/requestApproval",
        Some(json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "item-1"
        })),
    )?;
    let input = CodeApprovalResponseInput {
        runtime_generation: 1,
        request_id: CodeRequestId::String("approval-1".to_string()),
        scope: binding_scope(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
        response: CodeApprovalResponse::Decision {
            decision: CodeApprovalDecision::Accept,
        },
    };

    let error = respond_to_pending_approval(&approvals, &input, |_, _| {
        Err("simulated app-server write failure".to_string())
    })
    .expect_err("first response must fail");
    assert!(error.contains("simulated app-server write failure"));
    assert_eq!(approvals.len(), 1);

    respond_to_pending_approval(&approvals, &input, |request_id, result| {
        assert_eq!(request_id, json!("approval-1"));
        assert_eq!(result, json!({ "decision": "accept" }));
        Ok(())
    })?;
    assert_eq!(approvals.len(), 0);
    assert!(respond_to_pending_approval(&approvals, &input, |_, _| Ok(())).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn turn_selection_lists_before_write_and_rejects_unadvertised_pair_without_turn_bytes(
) -> Result<(), String> {
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
  request_id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"model/list"'*)
      printf '%s\n' "{\"id\":$request_id,\"result\":{\"data\":[{\"id\":\"gpt-visible-id\",\"model\":\"gpt-visible\",\"displayName\":\"GPT Visible\",\"description\":\"Visible model\",\"hidden\":false,\"supportedReasoningEfforts\":[{\"reasoningEffort\":\"xhigh\",\"description\":\"Deep\"}],\"defaultReasoningEffort\":\"xhigh\",\"isDefault\":true}],\"nextCursor\":null}}"
      ;;
    *'"method":"turn/start"'*)
      printf '%s\n' "{\"id\":$request_id,\"result\":{\"turn\":{\"id\":\"turn-model\",\"status\":\"inProgress\"}}}"
      ;;
    *)
      printf '%s\n' "{\"id\":$request_id,\"error\":{\"code\":-32601,\"message\":\"unexpected method\"}}"
      ;;
  esac
done
"#,
    )?;
    let request_log_executable = executable.clone();
    let runtime = CodeRuntime::with_executable(executable);
    let workspace = tempfile::tempdir().map_err(|error| error.to_string())?;
    runtime.start(noop_emitter())?;

    let accepted = runtime.turn_start_at(
        CodeTurnStartInput {
            scope: binding_scope(),
            thread_id: "thread-model".to_string(),
            prompt: "Use the selected model".to_string(),
            model: Some("gpt-visible".to_string()),
            effort: Some("xhigh".to_string()),
        },
        &workspace.path().to_string_lossy(),
    )?;
    assert_eq!(accepted.id, "turn-model");

    let error = runtime
        .turn_start_at(
            CodeTurnStartInput {
                scope: binding_scope(),
                thread_id: "thread-other".to_string(),
                prompt: "Reject this effort".to_string(),
                model: Some("gpt-visible".to_string()),
                effort: Some("medium".to_string()),
            },
            &workspace.path().to_string_lossy(),
        )
        .expect_err("unadvertised effort must fail before turn/start");
    assert!(error.contains("not supported"));
    runtime.stop()?;

    let requests = recorded_requests(&request_log_executable)?;
    assert_eq!(
        requests
            .iter()
            .map(|request| request["method"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["model/list", "turn/start", "model/list"]
    );
    assert_eq!(requests[0]["params"]["includeHidden"], false);
    assert_eq!(requests[0]["params"]["limit"], 100);
    assert_eq!(requests_for_method(&requests, "turn/start").len(), 1);
    Ok(())
}

#[cfg(unix)]
#[test]
fn bridges_delta_approval_interrupt_and_reconnect_contract() -> Result<(), String> {
    let (_directory, executable) = fake_codex(
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "codex-cli 0.145.0"
  exit 0
fi
IFS= read -r initialize
printf '%s\n' '{"id":1,"result":{"userAgent":"codex-test","codexHome":"/tmp/codex-home","platformFamily":"unix","platformOs":"macos"}}'
IFS= read -r initialized
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$0.requests"
  request_id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  case "$line" in
    *'"method":"thread/start"'*)
      case "$line" in
        *'"approvalPolicy":"on-request"'*'"sandbox":"workspace-write"'*) ;;
        *) printf '%s\n' "{\"id\":$request_id,\"error\":{\"code\":-32602,\"message\":\"unsafe thread defaults\"}}"; continue ;;
      esac
      printf '%s\n' "{\"id\":$request_id,\"result\":{\"thread\":{\"id\":\"thread-1\"},\"model\":\"gpt-test\",\"reasoningEffort\":null,\"instructionSources\":[]}}"
      printf '%s\n' '{"method":"thread/started","params":{"thread":{"id":"thread-1"}}}'
      ;;
    *'"method":"thread/resume"'*)
      printf '%s\n' "{\"id\":$request_id,\"result\":{\"thread\":{\"id\":\"thread-1\",\"status\":{\"type\":\"idle\"},\"turns\":[{\"id\":\"past-turn\",\"status\":\"completed\",\"items\":[{\"type\":\"agentMessage\",\"text\":\"restored\"}],\"error\":null}]},\"model\":\"gpt-test\",\"reasoningEffort\":\"high\",\"instructionSources\":[]}}"
      ;;
    *'"method":"thread/name/set"'*)
      printf '%s\n' "{\"id\":$request_id,\"result\":{}}"
      ;;
    *'"method":"thread/read"'*)
      printf '%s\n' "{\"id\":$request_id,\"result\":{\"thread\":{\"id\":\"thread-1\",\"cwd\":\"/native/stored-root\",\"name\":\"Renamed native contract\"}}}"
      ;;
    *'"method":"turn/start"'*)
      case "$line" in
        *'"approvalPolicy":"on-request"'*'"networkAccess":false'*'"type":"workspaceWrite"'*) ;;
        *) printf '%s\n' "{\"id\":$request_id,\"error\":{\"code\":-32602,\"message\":\"unsafe turn defaults\"}}"; continue ;;
      esac
      printf '%s\n' "{\"id\":$request_id,\"result\":{\"turn\":{\"id\":\"turn-1\",\"status\":\"inProgress\"}}}"
      printf '%s\n' '{"method":"turn/started","params":{"threadId":"thread-1","turn":{"id":"turn-1"}}}'
      printf '%s\n' '{"method":"item/agentMessage/delta","params":{"threadId":"thread-1","turnId":"turn-1","itemId":"item-1","delta":"working"}}'
      printf '%s\n' '{"id":"approval-1","method":"item/commandExecution/requestApproval","params":{"threadId":"thread-1","turnId":"turn-1","itemId":"item-2","startedAtMs":1,"command":"cargo test","cwd":"/tmp"}}'
      ;;
    *'"method":"turn/steer"'*)
      printf '%s\n' "{\"id\":$request_id,\"result\":{\"turnId\":\"turn-1\"}}"
      ;;
    *'"id":"approval-1"'*)
      printf '%s\n' '{"method":"serverRequest/resolved","params":{"threadId":"thread-1","requestId":"approval-1"}}'
      ;;
    *'"method":"turn/interrupt"'*)
      printf '%s\n' "{\"id\":$request_id,\"result\":{}}"
      printf '%s\n' '{"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"interrupted"}}}'
      ;;
    *)
      printf '%s\n' "{\"id\":$request_id,\"error\":{\"code\":-32601,\"message\":\"unexpected method\"}}"
      ;;
  esac
done
"#,
    )?;
    let workspace = tempfile::tempdir().map_err(|error| error.to_string())?;
    let uncanonical_workspace_root = format!("{}/.", workspace.path().to_string_lossy());
    let workspace_root = workspace
        .path()
        .canonicalize()
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .into_owned();
    let request_log_executable = executable.clone();
    let runtime = CodeRuntime::with_executable(executable);
    let scope = binding_scope();

    let first = runtime.start(noop_emitter())?;
    let opened = runtime
        .thread_start_at(
            CodeThreadStartInput {
                scope: scope.clone(),
                preparation_id: "67f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
                model: None,
            },
            &uncanonical_workspace_root,
        )
        .map_err(CodeThreadStartRpcError::into_message)?;
    assert_eq!(opened.thread.id, "thread-1");
    let resumed = runtime.thread_resume_at(
        CodeThreadResumeInput {
            scope: scope.clone(),
            thread_id: "thread-1".to_string(),
            model: None,
        },
        &uncanonical_workspace_root,
    )?;
    assert_eq!(resumed.thread.id, "thread-1");
    assert_eq!(resumed.thread.turns.len(), 1);
    assert_eq!(resumed.thread.turns[0].id, "past-turn");
    let read = runtime.thread_read("thread-1")?;
    assert_eq!(read.id, "thread-1");
    assert_eq!(read.cwd.as_deref(), Some("/native/stored-root"));
    let renamed = runtime.thread_rename(&CodeThreadRenameInput {
        scope: scope.clone(),
        thread_id: "thread-1".to_string(),
        name: "Renamed native contract".to_string(),
    })?;
    assert_eq!(renamed.id, "thread-1");
    assert_eq!(renamed.cwd.as_deref(), Some("/native/stored-root"));
    assert_eq!(renamed.name.as_deref(), Some("Renamed native contract"));

    let turn = runtime.turn_start_at(
        CodeTurnStartInput {
            scope: scope.clone(),
            thread_id: "thread-1".to_string(),
            prompt: "Run the tests".to_string(),
            model: None,
            effort: None,
        },
        &uncanonical_workspace_root,
    )?;
    assert_eq!(turn.id, "turn-1");
    let steered = runtime.turn_steer(CodeTurnSteerInput {
        scope: scope.clone(),
        thread_id: "thread-1".to_string(),
        expected_turn_id: "turn-1".to_string(),
        prompt: "Start with unit tests".to_string(),
    })?;
    assert_eq!(steered.id, "turn-1");

    let delta = wait_for_event(&runtime, "item/agentMessage/delta")?;
    assert_eq!(delta.runtime_generation, first.generation);
    assert_eq!(delta.thread_id.as_deref(), Some("thread-1"));
    assert_eq!(delta.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(delta.item_id.as_deref(), Some("item-1"));
    let approval = wait_for_event(&runtime, "item/commandExecution/requestApproval")?;
    assert!(approval.sequence > delta.sequence);

    runtime.approval_respond(CodeApprovalResponseInput {
        runtime_generation: first.generation,
        request_id: CodeRequestId::String("approval-1".to_string()),
        scope: scope.clone(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
        response: CodeApprovalResponse::Decision {
            decision: CodeApprovalDecision::Accept,
        },
    })?;
    let resolved = wait_for_event(&runtime, "serverRequest/resolved")?;
    assert!(resolved.sequence > approval.sequence);

    runtime.turn_interrupt(CodeTurnInterruptInput {
        scope: scope.clone(),
        thread_id: "thread-1".to_string(),
        turn_id: "turn-1".to_string(),
    })?;
    let completed = wait_for_event(&runtime, "turn/completed")?;
    assert!(completed.sequence > resolved.sequence);
    let first_snapshot = runtime.events(Some(first.generation), Some(delta.sequence))?;
    assert!(first_snapshot
        .events
        .iter()
        .all(|event| event.sequence > delta.sequence));
    assert!(first_snapshot
        .events
        .windows(2)
        .all(|events| events[0].sequence < events[1].sequence));

    runtime.stop()?;
    let second = runtime.start(noop_emitter())?;
    assert!(second.generation > first.generation);
    let reconnect = runtime.events(Some(first.generation), Some(completed.sequence))?;
    assert_eq!(reconnect.runtime_generation, second.generation);
    assert!(reconnect.truncated);
    assert!(reconnect.events.is_empty());
    let resumed_after_reconnect = runtime.thread_resume_at(
        CodeThreadResumeInput {
            scope,
            thread_id: "thread-1".to_string(),
            model: None,
        },
        &uncanonical_workspace_root,
    )?;
    assert_eq!(resumed_after_reconnect.thread.id, "thread-1");
    runtime.stop()?;

    let requests = recorded_requests(&request_log_executable)?;
    for request in &requests {
        assert!(request["params"].get("runtimeWorkspaceRoots").is_none());
        assert!(request["params"].get("scope").is_none());
        assert!(request["params"].get("communityId").is_none());
        assert!(request["params"].get("projectDtag").is_none());
        assert!(request["params"].get("repositoryIdentity").is_none());
        assert!(request["params"].get("descriptor").is_none());
    }
    let thread_starts = requests_for_method(&requests, "thread/start");
    let thread_resumes = requests_for_method(&requests, "thread/resume");
    assert_eq!(thread_starts.len(), 1);
    assert_eq!(thread_resumes.len(), 2);
    for request in thread_starts.into_iter().chain(thread_resumes) {
        assert_eq!(request["params"]["cwd"], workspace_root);
    }
    let thread_reads = requests_for_method(&requests, "thread/read");
    assert_eq!(thread_reads.len(), 2);
    for thread_read in thread_reads {
        assert_eq!(
            thread_read["params"],
            json!({ "threadId": "thread-1", "includeTurns": false })
        );
    }
    let thread_name_sets = requests_for_method(&requests, "thread/name/set");
    assert_eq!(thread_name_sets.len(), 1);
    assert_eq!(
        thread_name_sets[0]["params"],
        json!({
            "threadId": "thread-1",
            "name": "Renamed native contract",
        })
    );
    let rename_index = requests
        .iter()
        .position(|request| request["method"] == "thread/name/set")
        .ok_or_else(|| "missing thread/name/set request".to_string())?;
    assert_eq!(
        requests
            .get(rename_index + 1)
            .map(|request| &request["method"]),
        Some(&json!("thread/read"))
    );
    let turn_requests = requests_for_method(&requests, "turn/start");
    assert_eq!(turn_requests.len(), 1);
    assert_eq!(turn_requests[0]["params"]["cwd"], workspace_root);
    assert_eq!(
        turn_requests[0]["params"]["sandboxPolicy"]["writableRoots"],
        json!([workspace_root])
    );
    Ok(())
}
