use super::*;

#[test]
fn tauri_command_input_enum_and_event_contract_is_exact() -> Result<(), String> {
    let contract = fixture(TAURI_CONTRACT)?;
    let expected_commands = [
        ("code_runtime_probe", &[][..]),
        ("code_runtime_start", &[][..]),
        ("code_runtime_stop", &[][..]),
        ("code_runtime_status", &[][..]),
        (
            "code_runtime_events",
            &["afterSequence", "runtimeGeneration", "scope"][..],
        ),
        ("code_models_list", &[][..]),
        ("code_model_selection_set", &["input"][..]),
        ("code_terminal_open", &["input", "onEvent"][..]),
        ("code_terminal_resize", &["input"][..]),
        ("code_terminal_stdin", &["input"][..]),
        ("code_terminal_terminate", &["input"][..]),
        ("code_repository_inspect", &["input"][..]),
        ("code_worktree_prepare", &["input"][..]),
        ("code_worktree_status", &["descriptor"][..]),
        ("code_worktrees_list", &["input"][..]),
        ("code_worktree_remove", &["input"][..]),
        ("code_thread_preparations_list", &["input"][..]),
        ("code_threads_list", &["input"][..]),
        ("code_thread_archive", &["input"][..]),
        ("code_thread_unarchive", &["input"][..]),
        ("code_thread_rename", &["input"][..]),
        ("code_thread_changes", &["input"][..]),
        ("code_thread_start", &["input"][..]),
        ("code_thread_fork", &["input"][..]),
        ("code_thread_binding_recover", &["input"][..]),
        ("code_thread_resume", &["input"][..]),
        ("code_turn_start", &["input"][..]),
        ("code_turn_steer", &["input"][..]),
        ("code_turn_interrupt", &["input"][..]),
        ("code_approval_respond", &["input"][..]),
        ("code_thread_git_status", &["input"][..]),
        ("code_thread_git_stage", &["input"][..]),
        ("code_thread_git_unstage", &["input"][..]),
        ("code_thread_git_commit", &["input"][..]),
        ("code_thread_git_reconcile", &["input"][..]),
        ("code_thread_git_acknowledge", &["input"][..]),
    ];
    let actual = contract["commands"]
        .as_array()
        .ok_or_else(|| "missing commands".to_string())?;
    assert_eq!(actual.len(), expected_commands.len());
    for (actual, (name, args)) in actual.iter().zip(expected_commands.iter()) {
        assert_eq!(actual["name"], *name);
        assert_eq!(actual["topLevelArgs"], json!(args));
        assert_eq!(
            command_arguments(name)?,
            args.iter()
                .map(|argument| argument.to_string())
                .collect::<Vec<_>>()
        );
    }
    assert_eq!(
        registered_code_commands()?,
        expected_commands
            .iter()
            .map(|(name, _)| name.to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(contract["eventName"], CODE_WORKSPACE_EVENT);
    assert_eq!(
        contract["enums"]["executionMode"],
        encode_values([CodeExecutionMode::Worktree, CodeExecutionMode::Local])?
    );
    assert_eq!(
        contract["enums"]["threadChangeStatus"],
        encode_values([
            CodeThreadChangeStatus::Added,
            CodeThreadChangeStatus::Modified,
            CodeThreadChangeStatus::Deleted,
            CodeThreadChangeStatus::TypeChanged,
            CodeThreadChangeStatus::Unmerged,
            CodeThreadChangeStatus::Untracked,
        ])?
    );
    assert_eq!(
        contract["enums"]["preparationState"],
        encode_values([
            CodeThreadPreparationState::Prepared,
            CodeThreadPreparationState::Starting,
        ])?
    );
    assert_eq!(
        contract["enums"]["preparationOperation"],
        encode_values([
            CodeThreadPreparationOperation::Start,
            CodeThreadPreparationOperation::Fork,
        ])?
    );
    assert_eq!(
        contract["enums"]["runtimePhase"],
        encode_values([
            CodeRuntimePhase::NotInstalled,
            CodeRuntimePhase::Stopped,
            CodeRuntimePhase::Starting,
            CodeRuntimePhase::Initializing,
            CodeRuntimePhase::Ready,
            CodeRuntimePhase::Stopping,
            CodeRuntimePhase::Failed,
        ])?
    );
    assert_eq!(
        contract["enums"]["approvalDecision"],
        encode_values([
            CodeApprovalDecision::Accept,
            CodeApprovalDecision::AcceptForSession,
            CodeApprovalDecision::Decline,
            CodeApprovalDecision::Cancel,
        ])?
    );
    assert_eq!(
        contract["enums"]["permissionScope"],
        encode_values([CodePermissionScope::Turn, CodePermissionScope::Session])?
    );
    assert_eq!(
        contract["enums"]["permissionIntent"],
        encode_values([CodePermissionIntent::Grant, CodePermissionIntent::Decline])?
    );
    assert_eq!(
        contract["enums"]["threadLifecycle"],
        encode_values([
            CodeThreadLifecycleStatus::Active,
            CodeThreadLifecycleStatus::Archiving,
            CodeThreadLifecycleStatus::Archived,
            CodeThreadLifecycleStatus::Unarchiving,
            CodeThreadLifecycleStatus::Unknown,
        ])?
    );
    assert_eq!(
        contract["enums"]["worktreeInventoryBlocker"],
        encode_values([
            CodeWorktreeInventoryBlocker::ActiveBinding,
            CodeWorktreeInventoryBlocker::LifecycleUnsettled,
            CodeWorktreeInventoryBlocker::UnfinishedPreparation,
            CodeWorktreeInventoryBlocker::LocalCheckout,
            CodeWorktreeInventoryBlocker::UnavailableRoot,
            CodeWorktreeInventoryBlocker::DirtyRoot,
            CodeWorktreeInventoryBlocker::BranchAttached,
            CodeWorktreeInventoryBlocker::HeadDrift,
            CodeWorktreeInventoryBlocker::MergeProofUnavailable,
        ])?
    );

    let inputs = &contract["strictInputs"];
    reject_unknown::<CodeTerminalOpenInput>(&inputs["terminalOpen"])?;
    reject_unknown::<CodeTerminalResizeInput>(&inputs["terminalResize"])?;
    reject_unknown::<CodeTerminalStdinInput>(&inputs["terminalStdin"])?;
    reject_unknown::<CodeTerminalTerminateInput>(&inputs["terminalTerminate"])?;
    reject_unknown::<CodeRepositoryInspectInput>(&inputs["repositoryInspect"])?;
    reject_unknown::<CodeWorktreePrepareCommandInput>(&inputs["worktreePrepare"])?;
    reject_unknown::<CodeWorktreesListInput>(&inputs["worktreesList"])?;
    assert_eq!(
        keys(&inputs["worktreesList"])?,
        ["scope"].into_iter().map(str::to_string).collect()
    );
    reject_unknown::<CodeWorktreeRemoveInput>(&inputs["worktreeRemove"])?;
    assert_eq!(
        keys(&inputs["worktreeRemove"])?,
        ["scope", "threadId"]
            .into_iter()
            .map(str::to_string)
            .collect()
    );
    reject_unknown::<CodeThreadPreparationListInput>(&inputs["threadPreparationList"])?;
    reject_unknown::<CodeThreadStartInput>(&inputs["threadStart"])?;
    reject_unknown::<CodeThreadForkInput>(&inputs["threadFork"])?;
    reject_unknown::<CodeThreadBindingRecoverInput>(&inputs["threadBindingRecover"])?;
    reject_unknown::<CodeThreadListInput>(&inputs["threadList"])?;
    reject_unknown::<CodeThreadLifecycleInput>(&inputs["threadArchive"])?;
    reject_unknown::<CodeThreadLifecycleInput>(&inputs["threadUnarchive"])?;
    reject_unknown::<CodeThreadRenameInput>(&inputs["threadRename"])?;
    reject_unknown::<CodeThreadChangesInput>(&inputs["threadChanges"])?;
    reject_unknown::<CodeThreadResumeInput>(&inputs["threadResume"])?;
    reject_unknown::<CodeModelSelection>(&inputs["modelSelection"])?;
    reject_unknown::<CodeTurnStartInput>(&inputs["turnStart"])?;
    reject_unknown::<CodeTurnSteerInput>(&inputs["turnSteer"])?;
    reject_unknown::<CodeTurnInterruptInput>(&inputs["turnInterrupt"])?;
    reject_unknown::<CodeApprovalResponseInput>(&inputs["approvalDecision"])?;
    reject_unknown::<CodeApprovalResponseInput>(&inputs["approvalPermissions"])?;
    let response_types = ["approvalDecision", "approvalPermissions"]
        .into_iter()
        .map(|fixture_key| {
            decode::<CodeApprovalResponseInput>(&inputs[fixture_key]).map(|input| {
                match input.response {
                    CodeApprovalResponse::Decision { .. } => "decision",
                    CodeApprovalResponse::Permissions { .. } => "permissions",
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        contract["enums"]["approvalResponseType"],
        json!(response_types)
    );
    assert_eq!(
        keys(&contract["invocations"]["runtimeEvents"])?,
        ["afterSequence", "runtimeGeneration", "scope"]
            .into_iter()
            .map(str::to_string)
            .collect()
    );
    let _: CodeWorktreeDescriptor =
        decode(&contract["invocations"]["worktreeStatus"]["descriptor"])?;

    let binding: CodeThreadBinding = decode(&contract["outputs"]["binding"])?;
    assert_eq!(
        serde_json::to_value(&binding).map_err(|error| error.to_string())?,
        contract["outputs"]["binding"]
    );
    assert_eq!(keys(&contract["outputs"]["binding"])?.len(), 8);
    let model_selection: CodeModelSelection = decode(&contract["outputs"]["modelSelection"])?;
    assert_eq!(
        serde_json::to_value(&model_selection).map_err(|error| error.to_string())?,
        contract["outputs"]["modelSelection"]
    );
    assert_eq!(keys(&contract["outputs"]["modelSelection"])?.len(), 2);
    let model_catalog = CodeModelsListResult {
        runtime_generation: 7,
        models: vec![
            CodeModelOption {
                id: "gpt-5.2-codex".to_string(),
                model: "gpt-5.2-codex".to_string(),
                display_name: "GPT-5.2 Codex".to_string(),
                description: "Coding model for agentic workflows".to_string(),
                is_default: true,
                default_reasoning_effort: "medium".to_string(),
                supported_reasoning_efforts: vec![
                    CodeReasoningEffortOption {
                        reasoning_effort: "medium".to_string(),
                        description: "Balanced reasoning for everyday tasks".to_string(),
                    },
                    CodeReasoningEffortOption {
                        reasoning_effort: "high".to_string(),
                        description: "Deeper reasoning for complex tasks".to_string(),
                    },
                ],
            },
            CodeModelOption {
                id: "codex-mini".to_string(),
                model: "codex-mini-latest".to_string(),
                display_name: "Codex Mini".to_string(),
                description: "Fast coding model for focused tasks".to_string(),
                is_default: false,
                default_reasoning_effort: "low".to_string(),
                supported_reasoning_efforts: vec![
                    CodeReasoningEffortOption {
                        reasoning_effort: "low".to_string(),
                        description: "Fast responses for straightforward tasks".to_string(),
                    },
                    CodeReasoningEffortOption {
                        reasoning_effort: "medium".to_string(),
                        description: "Balanced reasoning for everyday tasks".to_string(),
                    },
                ],
            },
        ],
        recent_selection: Some(CodeModelSelection {
            model: "gpt-5.2-codex".to_string(),
            reasoning_effort: "medium".to_string(),
        }),
    };
    assert_eq!(
        serde_json::to_value(model_catalog).map_err(|error| error.to_string())?,
        contract["outputs"]["modelCatalog"]
    );
    assert_eq!(keys(&contract["outputs"]["modelCatalog"])?.len(), 3);
    let preparation_list: Vec<crate::code_workspace::bindings::CodeThreadPreparation> =
        decode(&contract["outputs"]["preparationList"])?;
    assert_eq!(
        serde_json::to_value(preparation_list).map_err(|error| error.to_string())?,
        contract["outputs"]["preparationList"]
    );
    let inventory: Vec<CodeWorktreeInventoryRow> =
        decode(&contract["outputs"]["worktreeInventory"])?;
    assert_eq!(
        serde_json::to_value(&inventory).map_err(|error| error.to_string())?,
        contract["outputs"]["worktreeInventory"]
    );
    assert!(inventory.iter().all(|row| {
        row.descriptor.execution_mode == CodeExecutionMode::Worktree
            && row.descriptor.worktree_id.is_some()
            && row.preserved
            && row.can_remove == row.blockers.is_empty()
    }));
    assert!(matches!(
        inventory[0].inspection,
        CodeWorktreeInspection::Unavailable { .. }
    ));
    assert!(matches!(
        inventory[0].authority,
        CodeWorktreeInventoryAuthority::Binding { .. }
    ));
    reject_unknown::<CodeWorktreeRemovalReceipt>(&contract["outputs"]["worktreeRemovalReceipt"])?;
    let removal_receipt: CodeWorktreeRemovalReceipt =
        decode(&contract["outputs"]["worktreeRemovalReceipt"])?;
    assert_eq!(
        serde_json::to_value(removal_receipt).map_err(|error| error.to_string())?,
        contract["outputs"]["worktreeRemovalReceipt"]
    );
    assert_eq!(
        keys(&contract["outputs"]["worktreeRemovalReceipt"])?.len(),
        9
    );

    let probe = CodeRuntimeProbe {
        available: true,
        executable: Some("/usr/local/bin/codex".to_string()),
        version: Some("codex-cli 0.145.0".to_string()),
        error: None,
    };
    assert_eq!(
        serde_json::to_value(probe).map_err(|error| error.to_string())?,
        contract["outputs"]["runtimeProbe"]
    );
    let status = CodeRuntimeStatus {
        phase: crate::code_workspace::runtime::CodeRuntimePhase::Ready,
        generation: 7,
        executable: Some("/usr/local/bin/codex".to_string()),
        version: Some("codex-cli 0.145.0".to_string()),
        pid: Some(1234),
        user_agent: Some("codex-test".to_string()),
        codex_home: Some("/native/codex-home".to_string()),
        platform_family: Some("unix".to_string()),
        platform_os: Some("macos".to_string()),
        queued_notifications: 2,
        last_error: None,
    };
    assert_eq!(
        serde_json::to_value(status).map_err(|error| error.to_string())?,
        contract["outputs"]["runtimeStatus"]
    );

    let terminal_scope: CodeThreadBindingScope =
        decode(&contract["strictInputs"]["terminalOpen"]["scope"])?;
    let terminal_session = CodeTerminalSession {
        scope: terminal_scope.clone(),
        thread_id: "thread-1".to_string(),
        session_id: "d9b41c7a-0e12-4df2-8c19-7e5a6b3c2901".to_string(),
        cols: 120,
        rows: 32,
    };
    assert_eq!(
        serde_json::to_value(terminal_session).map_err(|error| error.to_string())?,
        contract["outputs"]["terminalSession"]
    );
    let terminal_output_event = CodeTerminalEvent::Output {
        scope: terminal_scope.clone(),
        thread_id: "thread-1".to_string(),
        session_id: "d9b41c7a-0e12-4df2-8c19-7e5a6b3c2901".to_string(),
        sequence: 1,
        data: vec![36, 32],
    };
    assert_eq!(
        serde_json::to_value(terminal_output_event).map_err(|error| error.to_string())?,
        contract["outputs"]["terminalOutputEvent"]
    );
    let terminal_exit_event = CodeTerminalEvent::Exit {
        scope: terminal_scope,
        thread_id: "thread-1".to_string(),
        session_id: "d9b41c7a-0e12-4df2-8c19-7e5a6b3c2901".to_string(),
        sequence: 2,
        exit_code: 0,
        signal: None,
    };
    assert_eq!(
        serde_json::to_value(terminal_exit_event).map_err(|error| error.to_string())?,
        contract["outputs"]["terminalExitEvent"]
    );

    let repository_descriptor = CodeRepositoryDescriptor {
        repository_root: "/native/repository".to_string(),
        git_common_dir: "/native/repository/.git".to_string(),
        repository_identity: "a".repeat(64),
    };
    assert_eq!(
        serde_json::to_value(&repository_descriptor).map_err(|error| error.to_string())?,
        contract["outputs"]["repositoryDescriptor"]
    );
    let descriptor = CodeWorktreeDescriptor {
        execution_mode: crate::code_workspace::bindings::CodeExecutionMode::Local,
        repository_identity: "a".repeat(64),
        execution_root: "/native/stored-root".to_string(),
        base_ref: "b".repeat(40),
        worktree_id: None,
    };
    let worktree = CodeWorktreePrepareResult {
        repository: repository_descriptor,
        descriptor: descriptor.clone(),
        head_commit: "b".repeat(40),
        branch: Some("main".to_string()),
        dirty: false,
    };
    let prepared = CodePreparedWorktree {
        preparation_id: "67f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
        scope: decode(&contract["strictInputs"]["threadList"]["scope"])?,
        worktree: worktree.clone(),
    };
    assert_eq!(
        serde_json::to_value(prepared).map_err(|error| error.to_string())?,
        contract["outputs"]["preparedWorktree"]
    );
    let worktree_status = CodeWorktreeStatus {
        descriptor,
        head_commit: "b".repeat(40),
        branch: Some("main".to_string()),
        dirty: false,
    };
    assert_eq!(
        serde_json::to_value(worktree_status).map_err(|error| error.to_string())?,
        contract["outputs"]["worktreeStatus"]
    );

    let wire = fixture(WIRE_FIXTURE)?;
    let mut thread = parse_thread_open(wire["threadResume"]["result"].clone())?.thread;
    thread.name = Some("Native contract".to_string());
    assert_eq!(
        serde_json::to_value(&thread).map_err(|error| error.to_string())?,
        contract["outputs"]["threadSummary"]
    );
    let threads_page = CodeThreadsPage {
        data: vec![CodeBoundThreadSummary {
            binding: binding.clone(),
            lifecycle: CodeThreadLifecycleStatus::Active,
            thread: None,
            unavailable: Some("Codex app-server is not ready".to_string()),
        }],
        next_cursor: None,
        backwards_cursor: None,
    };
    assert_eq!(
        serde_json::to_value(threads_page).map_err(|error| error.to_string())?,
        contract["outputs"]["threadsPage"]
    );
    let lifecycle_mutation = CodeThreadLifecycleMutationResult {
        binding: binding.clone(),
        lifecycle: CodeThreadLifecycleStatus::Archived,
        thread: None,
    };
    assert_eq!(
        serde_json::to_value(lifecycle_mutation).map_err(|error| error.to_string())?,
        contract["outputs"]["threadLifecycleMutation"]
    );
    let thread_changes = CodeThreadChanges {
        files: vec![CodeThreadChangedFile {
            path: "desktop/src/features/code/ui/CodeChangesPanel.tsx".to_string(),
            status: CodeThreadChangeStatus::Modified,
            binary: false,
            additions: 12,
            deletions: 2,
            patch: "@@ -1,2 +1,3 @@\n-old\n+new\n+line".to_string(),
            truncated: false,
        }],
        total_files: 1,
        files_truncated: false,
        additions: 12,
        deletions: 2,
        commit_body: None,
    };
    assert_eq!(
        serde_json::to_value(thread_changes).map_err(|error| error.to_string())?,
        contract["outputs"]["threadChanges"]
    );
    let open = CodeBoundThreadOpenResult {
        binding: binding.clone(),
        thread,
        instruction_sources: vec!["AGENTS.md".to_string()],
        model: "gpt-5.2-codex".to_string(),
        reasoning_effort: Some("medium".to_string()),
    };
    assert_eq!(
        serde_json::to_value(open).map_err(|error| error.to_string())?,
        contract["outputs"]["boundThreadOpen"]
    );
    assert_eq!(keys(&contract["outputs"]["boundThreadOpen"])?.len(), 5);

    let event = CodeWorkspaceEvent {
        scope: decode(&contract["outputs"]["event"]["scope"])?,
        runtime_generation: 7,
        sequence: 11,
        thread_id: Some("thread-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        item_id: Some("item-1".to_string()),
        kind: "item/agentMessage/delta".to_string(),
        payload: contract["outputs"]["event"]["payload"].clone(),
    };
    assert_eq!(
        serde_json::to_value(&event).map_err(|error| error.to_string())?,
        contract["outputs"]["event"]
    );
    assert_eq!(keys(&contract["outputs"]["event"])?.len(), 8);
    let event_without_ids = CodeWorkspaceEvent {
        scope: decode(&contract["outputs"]["eventWithoutIds"]["scope"])?,
        runtime_generation: 7,
        sequence: 12,
        thread_id: None,
        turn_id: None,
        item_id: None,
        kind: "configWarning".to_string(),
        payload: contract["outputs"]["eventWithoutIds"]["payload"].clone(),
    };
    assert_eq!(
        serde_json::to_value(event_without_ids).map_err(|error| error.to_string())?,
        contract["outputs"]["eventWithoutIds"]
    );
    let backlog = CodeEventBacklog {
        runtime_generation: 7,
        latest_sequence: 11,
        truncated: false,
        checkpoint: Some(CodeEventCheckpoint {
            runtime_generation: 7,
            sequence_watermark: 11,
            active_turns: Vec::new(),
            pending_approvals: Vec::new(),
        }),
        events: vec![event],
    };
    assert_eq!(
        serde_json::to_value(backlog).map_err(|error| error.to_string())?,
        contract["outputs"]["eventBacklog"]
    );
    let turn = CodeTurnSummary {
        id: "turn-1".to_string(),
        status: "inProgress".to_string(),
    };
    assert_eq!(
        serde_json::to_value(turn).map_err(|error| error.to_string())?,
        contract["outputs"]["turnSummary"]
    );
    let start_error = CodeThreadStartError::recovery(
        "threadStartUncertain",
        "Codex response was interrupted".to_string(),
        "67f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
        None,
        Some("/native/stored-root".to_string()),
    );
    assert_eq!(
        serde_json::to_value(start_error).map_err(|error| error.to_string())?,
        contract["outputs"]["threadStartError"]
    );
    assert_eq!(
        serde_json::to_value(()).map_err(|error| error.to_string())?,
        contract["outputs"]["unitResponse"]
    );
    Ok(())
}
