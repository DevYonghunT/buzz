use super::*;

fn scope() -> CodeThreadBindingScope {
    CodeThreadBindingScope {
        community_id: "community-1".to_string(),
        project_dtag: "project-1".to_string(),
        repository_identity: "a".repeat(64),
    }
}

#[test]
fn thread_start_uses_only_the_native_workspace_root_and_stable_fields() -> Result<(), String> {
    let input = CodeThreadStartInput {
        scope: scope(),
        preparation_id: "67f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
        model: Some("test-model".to_string()),
    };

    let params = input.rpc_params("/native/stored-root")?;

    assert_eq!(params["cwd"], "/native/stored-root");
    assert_eq!(params["approvalPolicy"], "on-request");
    assert_eq!(params["sandbox"], "workspace-write");
    assert_eq!(params["model"], "test-model");
    assert_eq!(
        params["threadSource"],
        "schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb"
    );
    assert!(params.get("scope").is_none());
    assert!(params.get("preparationId").is_none());
    assert!(params.get("descriptor").is_none());
    assert!(params.get("workspaceRoot").is_none());
    assert!(params.get("runtimeWorkspaceRoots").is_none());
    assert_ne!(
        code_thread_source("67f11a1d-0274-4d40-9b0c-e406e51c64fb")?,
        code_thread_source("77f11a1d-0274-4d40-9b0c-e406e51c64fb")?
    );
    validate_code_thread_source_marker("schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb")?;
    for invalid in [
        "schoolx-code/not-a-uuid",
        "schoolx-code/67F11A1D-0274-4D40-9B0C-E406E51C64FB",
        "foreign/67f11a1d-0274-4d40-9b0c-e406e51c64fb",
    ] {
        assert!(validate_code_thread_source_marker(invalid).is_err());
    }
    Ok(())
}

#[test]
fn thread_fork_uses_only_the_exact_non_experimental_fields() -> Result<(), String> {
    let input = CodeThreadForkInput {
        scope: scope(),
        thread_id: "thread-source".to_string(),
    };

    let params = input.rpc_params(
        "/native/fork-destination",
        "67f11a1d-0274-4d40-9b0c-e406e51c64fb",
    )?;

    assert_eq!(
        params,
        json!({
            "threadId": "thread-source",
            "cwd": "/native/fork-destination",
            "approvalPolicy": "on-request",
            "sandbox": "workspace-write",
            "threadSource": "schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb"
        })
    );
    for forbidden in [
        "lastTurnId",
        "serviceName",
        "ephemeral",
        "model",
        "modelProvider",
        "config",
        "baseInstructions",
        "developerInstructions",
        "scope",
        "workspaceRoot",
    ] {
        assert!(params.get(forbidden).is_none(), "unexpected {forbidden}");
    }

    let caller_field = serde_json::from_value::<CodeThreadForkInput>(json!({
        "scope": scope(),
        "threadId": "thread-source",
        "lastTurnId": "caller-turn"
    }));
    assert!(caller_field.is_err());
    Ok(())
}

#[test]
fn thread_resume_uses_only_the_native_workspace_root_and_stable_fields() -> Result<(), String> {
    let input = CodeThreadResumeInput {
        scope: scope(),
        thread_id: "thread-1".to_string(),
        model: None,
    };

    let params = input.rpc_params("/native/stored-root")?;

    assert_eq!(params["threadId"], "thread-1");
    assert_eq!(params["cwd"], "/native/stored-root");
    assert_eq!(params["approvalPolicy"], "on-request");
    assert_eq!(params["sandbox"], "workspace-write");
    assert!(params.get("scope").is_none());
    assert!(params.get("workspaceRoot").is_none());
    assert!(params.get("runtimeWorkspaceRoots").is_none());
    Ok(())
}

#[test]
fn turn_start_confines_cwd_and_writable_roots_to_the_native_root() -> Result<(), String> {
    let input = CodeTurnStartInput {
        scope: scope(),
        thread_id: "thread-1".to_string(),
        prompt: "Run the tests".to_string(),
        model: None,
        effort: None,
    };

    let params = input.rpc_params("/native/stored-root")?;

    assert_eq!(params["cwd"], "/native/stored-root");
    assert_eq!(
        params["sandboxPolicy"]["writableRoots"],
        json!(["/native/stored-root"])
    );
    assert_eq!(params["sandboxPolicy"]["networkAccess"], false);
    assert!(params.get("scope").is_none());
    assert!(params.get("workspaceRoot").is_none());
    assert!(params.get("runtimeWorkspaceRoots").is_none());
    Ok(())
}

#[test]
fn thread_open_preserves_nullable_reasoning_effort() -> Result<(), String> {
    for response in [
        json!({"thread": {"id": "thread-1"}, "model": "gpt-test"}),
        json!({
            "thread": {"id": "thread-1"},
            "model": "gpt-test",
            "reasoningEffort": null
        }),
    ] {
        let opened = parse_thread_open(response)?;
        assert_eq!(opened.model, "gpt-test");
        assert_eq!(opened.reasoning_effort, None);
    }

    let opened = parse_thread_open(json!({
        "thread": {"id": "thread-1"},
        "model": "gpt-test",
        "reasoningEffort": "xhigh"
    }))?;
    assert_eq!(opened.model, "gpt-test");
    assert_eq!(opened.reasoning_effort.as_deref(), Some("xhigh"));
    Ok(())
}

#[test]
fn scoped_inputs_reject_the_removed_workspace_root_field() {
    let decoded = serde_json::from_value::<CodeThreadResumeInput>(json!({
        "scope": scope(),
        "threadId": "thread-1",
        "workspaceRoot": "/caller/root"
    }));

    assert!(decoded.is_err());
}

#[test]
fn recovery_input_rejects_a_caller_supplied_thread_id() {
    let decoded = serde_json::from_value::<CodeThreadBindingRecoverInput>(json!({
        "scope": scope(),
        "preparationId": "67f11a1d-0274-4d40-9b0c-e406e51c64fb",
        "threadId": "caller-selected-thread"
    }));

    assert!(decoded.is_err());
}

#[test]
fn builds_and_parses_exact_root_recovery_list_contract() -> Result<(), String> {
    assert_eq!(
        recovery_thread_list_params("/native/stored-root", Some("next-page"))?,
        json!({
            "sourceKinds": ["appServer", "vscode"],
            "archived": false,
            "cursor": "next-page",
            "cwd": "/native/stored-root",
            "limit": 100,
            "useStateDbOnly": false,
            "sortDirection": "desc",
            "sortKey": "created_at"
        })
    );

    let page = parse_recovery_thread_list(json!({
        "data": [{
            "id": "thread-1",
            "cwd": "/native/stored-root",
            "threadSource": "schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb"
        }],
        "nextCursor": "page-2",
        "backwardsCursor": "ignored"
    }))?;
    assert_eq!(page.data.len(), 1);
    assert_eq!(page.data[0].thread.id, "thread-1");
    assert_eq!(
        page.data[0].thread.cwd.as_deref(),
        Some("/native/stored-root")
    );
    assert_eq!(
        page.data[0].thread_source.as_deref(),
        Some("schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb")
    );
    assert_eq!(page.next_cursor.as_deref(), Some("page-2"));
    Ok(())
}

#[test]
fn parses_loaded_and_read_recovery_contracts() -> Result<(), String> {
    assert_eq!(
        loaded_thread_list_params(Some("loaded-next"))?,
        json!({ "limit": 100, "cursor": "loaded-next" })
    );
    let loaded = parse_loaded_thread_list(json!({
        "data": ["thread-1", "thread-2"],
        "nextCursor": null
    }))?;
    assert_eq!(loaded.data, vec!["thread-1", "thread-2"]);
    assert_eq!(loaded.next_cursor, None);

    let recovered = parse_recovery_thread_read(json!({
        "thread": {
            "id": "thread-1",
            "cwd": "/native/stored-root",
            "threadSource": "schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb"
        }
    }))?;
    assert_eq!(recovered.thread.id, "thread-1");
    assert_eq!(
        recovered.thread_source.as_deref(),
        Some("schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb")
    );
    Ok(())
}

#[test]
fn builds_and_parses_thread_read_contract() -> Result<(), String> {
    assert_eq!(
        thread_read_params("thread-1")?,
        json!({ "threadId": "thread-1", "includeTurns": false })
    );

    let thread = parse_thread_read(json!({
        "thread": {
            "id": "thread-1",
            "cwd": "/native/stored-root",
            "preview": "Continue the native work"
        }
    }))?;
    assert_eq!(thread.id, "thread-1");
    assert_eq!(thread.cwd.as_deref(), Some("/native/stored-root"));
    assert_eq!(thread.preview.as_deref(), Some("Continue the native work"));
    assert!(thread.turns.is_empty());
    Ok(())
}

#[test]
fn tauri_event_serializes_with_its_exact_binding_scope() -> Result<(), String> {
    let event = CodeRuntimeEvent {
        runtime_generation: 7,
        sequence: 11,
        thread_id: Some("thread-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        item_id: None,
        kind: "turn/started".to_string(),
        payload: json!({ "threadId": "thread-1" }),
    }
    .into_scoped(scope())
    .ok_or_else(|| "expected a thread-bound event".to_string())?;

    let value = serde_json::to_value(event).map_err(|error| error.to_string())?;
    assert_eq!(value["scope"]["communityId"], "community-1");
    assert_eq!(value["scope"]["projectDtag"], "project-1");
    assert_eq!(value["scope"]["repositoryIdentity"], "a".repeat(64));
    assert_eq!(value["runtimeGeneration"], 7);
    assert_eq!(value["sequence"], 11);
    assert_eq!(value["threadId"], "thread-1");
    Ok(())
}

#[test]
fn redacts_secret_shaped_text_in_rpc_diagnostics() {
    let redacted = redact_protocol_text("request failed with sk-project-secret");
    assert!(!redacted.contains("sk-project-secret"));
    assert!(redacted.contains("[REDACTED]"));
}

#[test]
fn redacts_injected_sensitive_env_values_without_mutating_process_env() {
    let canary = "schoolx-arbitrary-sensitive-env-canary";
    let redacted = redact_protocol_text_with_sensitive_values(
        &format!("probe.error={canary} start.error={canary} status.lastError={canary}"),
        &[canary],
    );

    assert!(!redacted.contains(canary));
    assert_eq!(redacted.matches("[REDACTED]").count(), 3);
}

#[test]
fn normalizes_delta_ids_and_redacts_known_secret_shapes() -> Result<(), String> {
    let event = normalize_notification(
        "item/agentMessage/delta",
        Some(json!({
            "threadId": "thread-1",
            "turnId": "turn-1",
            "itemId": "item-1",
            "delta": "tokens ghp_abc123456789 and sk-project-secret",
            "accessToken": "arbitrary-value"
        })),
    )?
    .ok_or_else(|| "expected a supported notification".to_string())?;

    assert_eq!(event.thread_id.as_deref(), Some("thread-1"));
    assert_eq!(event.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(event.item_id.as_deref(), Some("item-1"));
    assert_eq!(event.payload["delta"], "tokens [REDACTED] and [REDACTED]");
    assert_eq!(event.payload["accessToken"], "[REDACTED]");
    Ok(())
}

#[test]
fn lifecycle_notifications_are_strict_refetch_signals() -> Result<(), String> {
    for method in ["thread/archived", "thread/unarchived"] {
        let event = normalize_notification(method, Some(json!({ "threadId": "thread-1" })))?
            .ok_or_else(|| "expected a lifecycle notification".to_string())?;
        assert_eq!(event.thread_id.as_deref(), Some("thread-1"));
        assert!(event.turn_id.is_none());
        assert!(event.item_id.is_none());
        assert_eq!(event.kind, method);
        assert!(normalize_notification(method, Some(json!({}))).is_err());
        assert!(normalize_notification(
            method,
            Some(json!({ "threadId": "thread-1", "cwd": "/forged" }))
        )
        .is_err());
    }
    Ok(())
}

#[test]
fn ignores_unknown_notifications() -> Result<(), String> {
    assert!(normalize_notification("future/event", Some(json!({})))?.is_none());
    Ok(())
}
