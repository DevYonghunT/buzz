use super::*;

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "manual audit: requires the pinned Codex 0.145.0 CLI"]
async fn pinned_codex_0_145_session_permission_round_trip_is_manual_only() -> Result<(), String> {
    let executable = crate::managed_agents::resolve_command("codex")
        .ok_or_else(|| "Codex CLI is not installed".to_string())?;
    let version = Command::new(&executable)
        .arg("--version")
        .output()
        .map_err(|error| error.to_string())?;
    if !version.status.success()
        || String::from_utf8_lossy(&version.stdout).trim() != "codex-cli 0.145.0"
    {
        return Err("permission round-trip requires exact codex-cli 0.145.0".to_string());
    }

    let request_permissions_arguments = serde_json::to_string(&json!({
        "reason": "Allow loopback permission probe",
        "permissions": {
            "network": {
                "enabled": true
            }
        }
    }))
    .map_err(|error| error.to_string())?;
    let zero_usage = json!({
        "input_tokens": 0,
        "input_tokens_details": null,
        "output_tokens": 0,
        "output_tokens_details": null,
        "total_tokens": 0
    });
    let permission_response = sse_response(&[
        json!({
            "type": "response.created",
            "response": { "id": "response-permission" }
        }),
        json!({
            "type": "response.output_item.done",
            "item": {
                "type": "function_call",
                "call_id": "call1",
                "name": "request_permissions",
                "arguments": request_permissions_arguments
            }
        }),
        json!({
            "type": "response.completed",
            "response": {
                "id": "response-permission",
                "usage": zero_usage.clone()
            }
        }),
    ])?;
    let completion_response = sse_response(&[
        json!({
            "type": "response.created",
            "response": { "id": "response-complete" }
        }),
        json!({
            "type": "response.output_item.done",
            "item": {
                "type": "message",
                "role": "assistant",
                "id": "message-complete",
                "content": [{ "type": "output_text", "text": "done" }]
            }
        }),
        json!({
            "type": "response.completed",
            "response": {
                "id": "response-complete",
                "usage": zero_usage
            }
        }),
    ])?;
    let server = MockResponsesServer::start(vec![permission_response, completion_response]).await?;

    let codex_home = tempfile::tempdir().map_err(|error| error.to_string())?;
    fs::write(
        codex_home.path().join("config.toml"),
        format!(
            r#"
model = "mock-model"
model_provider = "mock_provider"
check_for_update_on_startup = false

[model_providers.mock_provider]
name = "SchoolX Code loopback permission test"
base_url = "{}"
env_key = "CODEX_PERMISSION_PROBE_TOKEN"
wire_api = "responses"
requires_openai_auth = false
request_max_retries = 0
stream_max_retries = 0

[features]
request_permissions_tool = true
"#,
            server.base_url
        ),
    )
    .map_err(|error| error.to_string())?;
    let (_wrapper_directory, wrapper) = real_codex_wrapper(&executable, codex_home.path())?;
    let workspace = tempfile::tempdir().map_err(|error| error.to_string())?;
    let workspace_root = workspace.path().to_string_lossy().into_owned();
    let expected_codex_home = fs::canonicalize(codex_home.path())
        .map_err(|error| error.to_string())?
        .to_string_lossy()
        .into_owned();
    let runtime = CodeRuntime::with_executable(wrapper);
    let scope = binding_scope();

    let ready = runtime.start(noop_emitter())?;
    assert_eq!(ready.version.as_deref(), Some("codex-cli 0.145.0"));
    assert_eq!(
        ready.codex_home.as_deref(),
        Some(expected_codex_home.as_str())
    );

    let opened = runtime
        .thread_start_at(
            CodeThreadStartInput {
                scope: scope.clone(),
                preparation_id: "67f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
                model: None,
            },
            &workspace_root,
        )
        .map_err(CodeThreadStartRpcError::into_message)?;
    let turn = runtime.turn_start_at(
        CodeTurnStartInput {
            scope: scope.clone(),
            thread_id: opened.thread.id.clone(),
            prompt: "pick a directory".to_string(),
            model: None,
            effort: None,
        },
        &workspace_root,
    )?;

    let approval = wait_for_event_with_timeout(
        &runtime,
        "item/permissions/requestApproval",
        Duration::from_secs(10),
    )?;
    assert_eq!(approval.runtime_generation, ready.generation);
    assert_eq!(
        approval.thread_id.as_deref(),
        Some(opened.thread.id.as_str())
    );
    assert_eq!(approval.turn_id.as_deref(), Some(turn.id.as_str()));
    assert_eq!(approval.item_id.as_deref(), Some("call1"));
    assert_eq!(approval.payload["approvalKind"], "permissions");
    assert_eq!(
        approval.payload["request"]["reason"],
        "Allow loopback permission probe"
    );
    let request_id = CodeRequestId::from_value(approval.payload["requestId"].clone())?;
    let resolved_request_id = request_id.to_value();
    let requested_network = approval
        .payload
        .pointer("/request/permissionDisplay/network/enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| "permission request has no network grant".to_string())?;
    assert!(requested_network);

    runtime.approval_respond(CodeApprovalResponseInput {
        runtime_generation: ready.generation,
        request_id,
        scope,
        thread_id: opened.thread.id.clone(),
        turn_id: turn.id.clone(),
        response: CodeApprovalResponse::Permissions {
            intent: super::super::super::approvals::CodePermissionIntent::Grant,
            scope: CodePermissionScope::Session,
        },
    })?;
    let resolved =
        wait_for_event_with_timeout(&runtime, "serverRequest/resolved", Duration::from_secs(10))?;
    assert_eq!(
        resolved.thread_id.as_deref(),
        Some(opened.thread.id.as_str())
    );
    assert_eq!(resolved.payload["requestId"], resolved_request_id);
    let completed =
        wait_for_event_with_timeout(&runtime, "turn/completed", Duration::from_secs(10))?;
    assert!(approval.sequence < resolved.sequence);
    assert!(resolved.sequence < completed.sequence);
    runtime.stop()?;

    let requests = server.requests()?;
    assert_eq!(requests.len(), 2);
    assert!(requests[0]["tools"].as_array().is_some_and(|tools| {
        tools
            .iter()
            .any(|tool| tool["name"] == "request_permissions")
    }));
    let permission_output = requests[1]["input"]
        .as_array()
        .and_then(|input| {
            input
                .iter()
                .find(|item| item["type"] == "function_call_output" && item["call_id"] == "call1")
        })
        .and_then(|item| item["output"].as_str())
        .ok_or_else(|| "second model request has no permission tool output".to_string())?;
    let permission_output: Value =
        serde_json::from_str(permission_output).map_err(|error| error.to_string())?;
    assert_eq!(permission_output["permissions"]["network"]["enabled"], true);
    assert_eq!(permission_output["scope"], "session");
    assert_eq!(server.remaining_responses()?, 0);
    Ok(())
}
