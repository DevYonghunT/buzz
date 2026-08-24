use super::*;

fn assert_native_codex_builders_and_parsers_match_wire_fixture(
    wire_fixture: &str,
) -> Result<(), String> {
    let wire = fixture(wire_fixture)?;
    let contract = fixture(TAURI_CONTRACT)?;
    let inputs = &contract["strictInputs"];

    let start: CodeThreadStartInput = decode(&inputs["threadStart"])?;
    assert_eq!(
        start.rpc_params("/native/stored-root")?,
        wire["threadStart"]["params"]
    );
    let opened = parse_thread_open(wire["threadStart"]["result"].clone())?;
    assert_eq!(opened.thread.id, "thread-1");
    assert_eq!(opened.model, "gpt-5.2-codex");
    assert_eq!(opened.reasoning_effort.as_deref(), Some("medium"));
    assert_eq!(
        opened.thread_source.as_deref(),
        Some("schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb")
    );

    let fork: CodeThreadForkInput = decode(&inputs["threadFork"])?;
    assert_eq!(
        fork.rpc_params("/native/fork-root", "89c210f4-7a5b-4bd5-a98c-322386a8a2e9")?,
        wire["threadFork"]["params"]
    );
    assert_eq!(
        keys(&wire["threadFork"]["params"])?,
        [
            "approvalPolicy",
            "cwd",
            "sandbox",
            "threadId",
            "threadSource",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    );
    let forked = parse_thread_open(wire["threadFork"]["result"].clone())?;
    assert_eq!(forked.thread.id, "thread-2");
    assert_eq!(forked.model, "gpt-5.2-codex");
    assert_eq!(forked.reasoning_effort, None);
    assert_eq!(forked.thread.session_id.as_deref(), Some("thread-2"));
    assert_eq!(forked.thread.forked_from_id.as_deref(), Some("thread-1"));
    assert_eq!(forked.thread.parent_thread_id, None);
    assert!(!forked.thread.ephemeral);
    assert_eq!(forked.thread.cwd.as_deref(), Some("/native/fork-root"));
    assert_eq!(forked.response_cwd.as_deref(), Some("/native/fork-root"));
    assert_eq!(
        forked.thread_source.as_deref(),
        Some("schoolx-code/89c210f4-7a5b-4bd5-a98c-322386a8a2e9")
    );
    assert_eq!(forked.session_source, Some(json!("appServer")));
    assert_eq!(forked.thread.turns[0].id, "past-turn");

    assert_eq!(
        recovery_thread_list_params("/native/stored-root", Some("list-next"))?,
        wire["threadList"]["params"]
    );
    let listed = parse_recovery_thread_list(wire["threadList"]["result"].clone())?;
    assert_eq!(listed.data[0].thread.id, "thread-1");
    assert_eq!(listed.next_cursor.as_deref(), Some("list-page-2"));

    assert_eq!(
        loaded_thread_list_params(Some("loaded-next"))?,
        wire["threadLoadedList"]["params"]
    );
    let loaded = parse_loaded_thread_list(wire["threadLoadedList"]["result"].clone())?;
    assert_eq!(loaded.data, vec!["thread-1", "thread-2"]);

    assert_eq!(
        thread_read_params("thread-1")?,
        wire["threadRead"]["params"]
    );
    assert_eq!(
        parse_thread_read(wire["threadRead"]["result"].clone())?.id,
        "thread-1"
    );
    let recovered = parse_recovery_thread_read(wire["threadRead"]["result"].clone())?;
    assert_eq!(
        recovered.thread_source.as_deref(),
        Some("schoolx-code/67f11a1d-0274-4d40-9b0c-e406e51c64fb")
    );

    let rename: CodeThreadRenameInput = decode(&inputs["threadRename"])?;
    assert_eq!(rename.rpc_params()?, wire["threadNameSet"]["params"]);
    parse_thread_name_set(wire["threadNameSet"]["result"].clone())?;
    assert!(parse_thread_name_set(json!({ "unexpected": true })).is_err());

    let archive: CodeThreadLifecycleInput = decode(&inputs["threadArchive"])?;
    assert_eq!(archive.rpc_params()?, wire["threadArchive"]["params"]);
    parse_thread_archive(wire["threadArchive"]["result"].clone())?;
    assert!(parse_thread_archive(json!({ "unexpected": true })).is_err());

    let unarchive: CodeThreadLifecycleInput = decode(&inputs["threadUnarchive"])?;
    assert_eq!(unarchive.rpc_params()?, wire["threadUnarchive"]["params"]);
    assert_eq!(
        parse_thread_unarchive(wire["threadUnarchive"]["result"].clone())?.id,
        "thread-1"
    );

    let resume: CodeThreadResumeInput = decode(&inputs["threadResume"])?;
    assert_eq!(
        resume.rpc_params("/native/stored-root")?,
        wire["threadResume"]["params"]
    );
    let resumed = parse_thread_open(wire["threadResume"]["result"].clone())?;
    assert_eq!(resumed.thread.turns[0].id, "past-turn");
    assert_eq!(resumed.model, "gpt-5.2-codex");
    assert_eq!(resumed.reasoning_effort.as_deref(), Some("medium"));

    let turn: CodeTurnStartInput = decode(&inputs["turnStart"])?;
    assert_eq!(
        turn.rpc_params("/native/stored-root")?,
        wire["turnStart"]["params"]
    );
    assert_eq!(
        parse_turn_start(wire["turnStart"]["result"].clone())?.id,
        "turn-1"
    );

    let steer: CodeTurnSteerInput = decode(&inputs["turnSteer"])?;
    assert_eq!(steer.rpc_params()?, wire["turnSteer"]["params"]);
    assert_eq!(
        parse_turn_steer(wire["turnSteer"]["result"].clone())?.id,
        "turn-1"
    );

    let interrupt: CodeTurnInterruptInput = decode(&inputs["turnInterrupt"])?;
    assert_eq!(interrupt.rpc_params()?, wire["turnInterrupt"]["params"]);
    for params in [
        &wire["threadFork"]["params"],
        &wire["threadStart"]["params"],
        &wire["threadResume"]["params"],
        &wire["threadNameSet"]["params"],
        &wire["threadArchive"]["params"],
        &wire["threadUnarchive"]["params"],
        &wire["turnStart"]["params"],
        &wire["turnSteer"]["params"],
        &wire["turnInterrupt"]["params"],
    ] {
        for forbidden in [
            "scope",
            "communityId",
            "projectDtag",
            "repositoryIdentity",
            "workspaceRoot",
            "descriptor",
            "runtimeWorkspaceRoots",
        ] {
            assert!(params.get(forbidden).is_none(), "wire leaked {forbidden}");
        }
    }

    assert_eq!(initialize_params(), wire["initialize"]["request"]["params"]);
    let initialize = jsonrpc::request(1, "initialize", initialize_params());
    assert!(initialize.get("jsonrpc").is_none());
    assert_eq!(
        jsonrpc::notification("initialized"),
        wire["initialize"]["initializedNotification"]
    );
    assert!(wire["initialize"]["initializedNotification"]
        .get("jsonrpc")
        .is_none());
    Ok(())
}

#[test]
fn native_codex_builders_and_parsers_match_the_0_145_wire_fixture() -> Result<(), String> {
    assert_native_codex_builders_and_parsers_match_wire_fixture(WIRE_FIXTURE)
}

#[test]
fn native_codex_builders_and_parsers_match_the_0_149_wire_fixture() -> Result<(), String> {
    assert_native_codex_builders_and_parsers_match_wire_fixture(WIRE_FIXTURE_0_149)
}

fn assert_supported_notifications_and_approvals_match_wire_fixture(
    wire_fixture: &str,
) -> Result<(), String> {
    let wire = fixture(wire_fixture)?;
    let notifications = wire["notifications"]
        .as_array()
        .ok_or_else(|| "missing notifications".to_string())?;
    assert_eq!(notifications.len(), 23);
    let mut methods = HashSet::new();
    for notification in notifications {
        let method = notification["method"]
            .as_str()
            .ok_or_else(|| "notification has no method".to_string())?;
        assert!(methods.insert(method));
        let normalized = normalize_notification(method, Some(notification["params"].clone()))?
            .ok_or_else(|| format!("supported notification `{method}` was ignored"))?;
        assert_eq!(normalized.kind, method);
        assert_eq!(normalized.payload, notification["params"]);
        if let Some(thread_id) = notification["params"]
            .get("threadId")
            .and_then(Value::as_str)
        {
            assert_eq!(normalized.thread_id.as_deref(), Some(thread_id));
        }
    }
    assert!(normalize_notification("future/unknown", Some(json!({})))?.is_none());

    let approvals = PendingApprovalStore::default();
    approvals.reset(7);
    for approval in wire["approvals"]
        .as_array()
        .ok_or_else(|| "missing approvals".to_string())?
    {
        let request = &approval["request"];
        let event = approvals
            .insert_request(
                7,
                request["id"].clone(),
                request["method"].as_str().unwrap_or_default(),
                Some(request["params"].clone()),
            )?
            .ok_or_else(|| "approval request was not recognized".to_string())?;
        assert_eq!(event.kind, request["method"]);
        if request["method"] == "item/permissions/requestApproval" {
            let public_request = event.payload["request"]
                .as_object()
                .ok_or_else(|| "permission event request was not an object".to_string())?;
            assert!(!public_request.contains_key("permissions"));
            assert_eq!(public_request["permissionDisplay"]["grantable"], true);
        }
        let input: CodeApprovalResponseInput = decode(&approval["responseInput"])?;
        let reservation = approvals.reserve_response(&input)?;
        let (request_id, result) = reservation.wire_response();
        assert_eq!(request_id, request["id"]);
        assert_eq!(result, approval["wireResult"]);
        approvals.commit_response(&reservation)?;
    }
    assert_eq!(approvals.len(), 0);
    Ok(())
}

#[test]
fn all_supported_notifications_and_approvals_match_0_145_wire_fixtures() -> Result<(), String> {
    assert_supported_notifications_and_approvals_match_wire_fixture(WIRE_FIXTURE)
}

#[test]
fn all_supported_notifications_and_approvals_match_0_149_wire_fixtures() -> Result<(), String> {
    assert_supported_notifications_and_approvals_match_wire_fixture(WIRE_FIXTURE_0_149)
}

#[test]
#[ignore = "manual audit: requires an exact audited Codex 0.145.0 or 0.149.0 CLI"]
fn refresh_schema_snapshot_is_manual_only() -> Result<(), String> {
    let executable = crate::managed_agents::resolve_command("codex")
        .ok_or_else(|| "Codex CLI is not installed".to_string())?;
    let version = Command::new(&executable)
        .arg("--version")
        .output()
        .map_err(|error| error.to_string())?;
    if !version.status.success() {
        return Err("schema refresh Codex version probe failed".to_string());
    }
    let version = String::from_utf8_lossy(&version.stdout);
    let manifest = match version.trim() {
        "codex-cli 0.145.0" => fixture(SCHEMA_MANIFEST)?,
        "codex-cli 0.149.0" => fixture(SCHEMA_MANIFEST_0_149)?,
        _ => {
            return Err(
                "schema refresh requires exact codex-cli 0.145.0 or codex-cli 0.149.0".to_string(),
            )
        }
    };

    let generated = tempfile::tempdir().map_err(|error| error.to_string())?;
    let output = Command::new(executable)
        .args(["app-server", "generate-json-schema", "--out"])
        .arg(generated.path())
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "Codex schema generation failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let mut paths = Vec::new();
    collect_json_paths(generated.path(), generated.path(), &mut paths)?;
    paths.sort();
    assert_eq!(
        paths.len(),
        manifest["source"]["generatedFileCount"]
            .as_u64()
            .ok_or_else(|| "generatedFileCount must be numeric".to_string())? as usize
    );
    let selected = manifest["schemas"]
        .as_array()
        .ok_or_else(|| "manifest schemas must be an array".to_string())?
        .iter()
        .map(|entry| {
            let hash = entry[0]
                .as_str()
                .ok_or_else(|| "manifest schema hash must be a string".to_string())?;
            let path = entry[1]
                .as_str()
                .ok_or_else(|| "manifest schema path must be a string".to_string())?;
            Ok((path.to_string(), hash.to_string()))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let mut full_aggregate = String::new();
    let mut selected_aggregate = String::new();
    for path in paths {
        let schema = serde_json::from_slice(
            &fs::read(generated.path().join(&path)).map_err(|error| error.to_string())?,
        )
        .map_err(|error| format!("invalid generated schema {path}: {error}"))?;
        let hash = sha256_hex(&canonical_schema_bytes(schema)?);
        full_aggregate.push_str(&hash);
        full_aggregate.push_str("  ");
        full_aggregate.push_str(&path);
        full_aggregate.push('\n');
        if let Some(expected) = selected.get(&path) {
            assert_eq!(&hash, expected, "generated schema drifted for {path}");
            selected_aggregate.push_str(&hash);
            selected_aggregate.push_str("  ");
            selected_aggregate.push_str(&path);
            selected_aggregate.push('\n');
        }
    }
    assert_eq!(
        sha256_hex(full_aggregate.as_bytes()),
        manifest["source"]["fullGeneratedSetSha256"]
    );
    assert_eq!(
        sha256_hex(selected_aggregate.as_bytes()),
        manifest["source"]["selectedSchemasSha256"]
    );
    Ok(())
}
