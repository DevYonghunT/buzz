const GIT_WRITE_SOURCE: &str = include_str!("git_write/git_command.rs");
const GIT_WRITE_CAPTURE_SOURCE: &str = include_str!("git_write/git_command/capture.rs");
const PINNED_GIT_SOURCE: &str = include_str!("worktrees.rs");
const REMOVAL_GIT_SOURCE: &str = include_str!("bindings/removal/physical/unix.rs");
const LINUX_LAUNCH_SOURCE: &str = include_str!("git_launch.rs");
const MACOS_XPC_SOURCE: &str = include_str!("macos_git_xpc.rs");
const MACOS_XPC_RUST_SESSION_SOURCE: &str = include_str!("macos_git_xpc_session.rs");
const GIT_WRITE_ENGINE_SOURCE: &str = include_str!("git_write/engine.rs");
const GIT_WRITE_RECOVERY_SOURCE: &str = include_str!("git_write/transaction/recovery.rs");
const GIT_WRITE_STARTUP_SOURCE: &str = include_str!("git_write/startup.rs");
const PROJECT_GIT_EXEC_SOURCE: &str = include_str!("../commands/project_git_exec.rs");
const CODE_WORKSPACE_COMMAND_SOURCE: &str = include_str!("../commands/code_workspace.rs");
const CODE_GIT_HANDOFF_SOURCE: &str = include_str!("../commands/code_git_handoff.rs");
const MACOS_XPC_CLIENT_SOURCE: &str = include_str!("../../macos/SchoolXGitXpc.swift");
const MACOS_XPC_MESSAGES_SOURCE: &str = include_str!("../../macos/SchoolXGitXpcMessages.swift");
const MACOS_XPC_SERVICE_SOURCE: &str = include_str!("../../macos/SchoolXGitXpcService.swift");
const MACOS_XPC_LIFECYCLE_SOURCE: &str = include_str!("../../macos/SchoolXGitXpcLifecycle.swift");
const MACOS_XPC_SESSION_SOURCE: &str = include_str!("../../macos/SchoolXGitXpcSession.swift");
const MACOS_XPC_SUPPORT_SOURCE: &str = include_str!("../../macos/SchoolXGitXpcSupport.swift");
const WORKTREE_INVENTORY_SOURCE: &str = include_str!("worktree_inventory.rs");
const CODE_WORKSPACE_SCREEN_SOURCE: &str =
    include_str!("../../../src/features/code/ui/CodeWorkspaceScreen.tsx");
const TAURI_BUILD_SOURCE: &str = include_str!("../../build.rs");
const MAIN_SOURCE: &str = include_str!("../main.rs");

#[test]
fn production_code_git_self_reexec_dispatch_is_absent() {
    for source in [GIT_WRITE_SOURCE, PINNED_GIT_SOURCE, REMOVAL_GIT_SOURCE] {
        assert!(!source.contains("HELPER_ARGUMENT"));
        assert_eq!(source.matches("std::env::current_exe()").count(), 1);
    }

    assert!(GIT_WRITE_SOURCE
        .contains("#[cfg(all(unix, not(target_os = \"linux\"), test))]\nfn spawn_helper"));
    assert!(PINNED_GIT_SOURCE.contains(
        "#[cfg(all(unix, not(target_os = \"linux\"), test))]\nfn spawn_pinned_git_path_helper_child"
    ));
    assert!(REMOVAL_GIT_SOURCE
        .contains("#[cfg(all(not(target_os = \"linux\"), test))]\n    let mut captured"));

    assert!(GIT_WRITE_SOURCE.contains(
        "#[cfg(all(unix, test))]\nconst HELPER_REQUEST_ENV: &str = \"SCHOOLX_CODE_GIT_WRITE_REQUEST\";"
    ));
    assert!(PINNED_GIT_SOURCE.contains(
        "#[cfg(all(unix, test))]\nconst PINNED_GIT_REQUEST_ENV: &str = \"SCHOOLX_CODE_PINNED_GIT_REQUEST_V1\";"
    ));
    assert!(REMOVAL_GIT_SOURCE.contains(
        "#[cfg(test)]\nconst HELPER_ENV: &str = \"SCHOOLX_CODE_REMOVAL_GIT_REQUEST_V1\";"
    ));
}

#[test]
fn macos_xpc_service_dispatch_precedes_tauri_without_legacy_dispatch() {
    let xpc = MAIN_SOURCE
        .find("run_code_git_xpc_service_if_requested")
        .expect("macOS XPC service dispatch must remain in main");
    let tauri = MAIN_SOURCE
        .rfind("buzz_lib::run()")
        .expect("Tauri startup must remain in main");
    assert!(xpc < tauri);
    assert!(!MAIN_SOURCE.contains("run_code_pinned_git_helper_if_requested"));
}

#[test]
fn linux_launch_is_descriptor_bound_without_child_side_rust_callbacks() {
    for required in [
        "RootTrustedGit::pin()",
        "fcntl_dupfd_cloexec",
        "PROC_SUPER_MAGIC",
        "current_dir(cwd.path())",
        "process_group(0)",
        "arg(\"--version\")",
    ] {
        assert!(
            LINUX_LAUNCH_SOURCE.contains(required),
            "Linux launch contract lost {required}"
        );
    }
    for forbidden in ["pre_exec", "unsafe {"] {
        assert!(!LINUX_LAUNCH_SOURCE.contains(forbidden));
    }
}

#[test]
fn macos_rust_bridge_has_no_raw_spawn_or_child_side_callback() {
    assert!(MACOS_XPC_SOURCE.contains("ROOT_TRUSTED_MACOS_GIT"));
    assert!(MACOS_XPC_SOURCE.contains("MacGitAuthoritySession"));
    assert!(MACOS_XPC_SOURCE.contains("ACTIVE_SESSION_ID"));
    assert!(MACOS_XPC_SOURCE.contains("schoolx_git_xpc_session_begin"));
    assert!(MACOS_XPC_SOURCE.contains("schoolx_git_xpc_session_end"));
    assert!(MACOS_XPC_SOURCE.contains("schoolx_git_xpc_session_cleanup_proven"));
    for (path, source) in [
        ("macos_git_xpc.rs", MACOS_XPC_SOURCE),
        ("macos_git_xpc_session.rs", MACOS_XPC_RUST_SESSION_SOURCE),
    ] {
        assert!(
            source.lines().count() < 1_000,
            "{path} crossed the 1,000-line source ceiling"
        );
        for forbidden in ["pre_exec", "unsafe {"] {
            assert!(!source.contains(forbidden), "{path} contains {forbidden}");
        }
    }
}

#[test]
fn macos_xpc_session_is_kernel_leased_and_exactly_signed() {
    for required in [
        "protocolVersion: UInt64 = 3",
        "1.2.840.113635.100.6.2.6",
        "1.2.840.113635.100.6.1.13",
    ] {
        assert!(
            MACOS_XPC_SUPPORT_SOURCE.contains(required),
            "macOS signed-session contract lost {required}"
        );
    }
    for required in [
        "gitReservationFD",
        "LOCK_EX | LOCK_NB",
        "xpc_transaction_begin()",
        "sessionBegin",
        "sessionEnd",
        "sessionEnded",
        "sessionNonceHigh",
        "sessionNonceLow",
        "lateCleanupArmed",
        "schoolx_git_xpc_session_cleanup_proven",
    ] {
        assert!(
            MACOS_XPC_SESSION_SOURCE.contains(required),
            "macOS authority-session contract lost {required}"
        );
    }
    for required in ["messageMatchesSession", "cancelAck"] {
        assert!(MACOS_XPC_MESSAGES_SOURCE.contains(required));
    }
    for required in ["prepareOrphanedSessionEnd", "completeOrphanedSessionEnd"] {
        assert!(MACOS_XPC_SESSION_SOURCE.contains(required));
    }
    for forbidden in ["SIGKILL", "SIGTERM", "SIGSTOP", "SIGCONT"] {
        assert!(!MACOS_XPC_CLIENT_SOURCE.contains(forbidden));
    }
    assert!(MACOS_XPC_SUPPORT_SOURCE.contains("SIGCHLD"));
    assert!(MACOS_XPC_SUPPORT_SOURCE.contains("SIGKILL"));
}

#[test]
fn authenticated_macos_session_reuse_skips_redundant_static_code_validation() {
    let rust_begin = MACOS_XPC_SOURCE
        .find("pub(crate) fn begin() -> Result<Self, String>")
        .map(|offset| &MACOS_XPC_SOURCE[offset..])
        .expect("macOS authority-session begin must remain explicit");
    let ambient = rust_begin
        .find("let ambient = THREAD_SESSION")
        .expect("macOS nested authority must inspect its ambient session");
    let global_gate = rust_begin
        .find("ACTIVE_SESSION_ID\n            .compare_exchange")
        .expect("fresh macOS authority sessions must acquire the process-global gate");
    let signed_admission = rust_begin
        .find("schoolx_git_xpc_session_begin(session_id)")
        .expect("fresh macOS authority sessions must enter signed XPC admission");
    assert!(ambient < global_gate && global_gate < signed_admission);
    assert!(!MACOS_XPC_SOURCE.contains("schoolx_git_xpc_capability"));
    assert!(!MACOS_XPC_CLIENT_SOURCE.contains("schoolx_git_xpc_capability"));
    assert!(!GIT_WRITE_SOURCE.contains("macos_git_xpc::require_capability()"));
    assert!(!PINNED_GIT_SOURCE.contains("macos_git_xpc::require_capability()"));

    let session_begin = MACOS_XPC_SESSION_SOURCE
        .find("func schoolx_git_xpc_session_begin")
        .map(|offset| &MACOS_XPC_SESSION_SOURCE[offset..])
        .expect("macOS XPC session admission must remain explicit");
    let session_begin_end = session_begin
        .find("\nfunc schoolx_git_xpc_session_end")
        .expect("macOS XPC session admission boundary must remain explicit");
    let session_begin = &session_begin[..session_begin_end];
    assert_eq!(session_begin.matches("capabilityDiagnostic()").count(), 1);
    assert!(session_begin.contains("installPeerRequirement("));
    let capability = session_begin
        .find("capabilityDiagnostic()")
        .expect("fresh session must validate the signed helper");
    let reservation = session_begin
        .find("openGitReservation()")
        .expect("fresh session must reserve root-trusted Git");
    let peer_requirement = session_begin
        .find("installPeerRequirement(")
        .expect("fresh session must authenticate its XPC peer");
    let activation = session_begin
        .find("xpc_connection_activate(connection)")
        .expect("fresh session must activate its authenticated XPC connection");
    assert!(capability < reservation && reservation < peer_requirement);
    assert!(peer_requirement < activation);
    assert!(
        session_begin.contains("return encodeSessionFailure(sessionID: session_id, diagnostic)")
    );
    let encoded_failure = MACOS_XPC_SESSION_SOURCE
        .find("private func encodeSessionFailure")
        .map(|offset| &MACOS_XPC_SESSION_SOURCE[offset..])
        .expect("clean XPC admission failures must have an explicit disposition");
    let encoded_failure_end = encoded_failure
        .find("\nprivate func encodeSessionRetained")
        .expect("clean and retained XPC dispositions must remain separate");
    let encoded_failure = &encoded_failure[..encoded_failure_end];
    for required in [
        "sessionId: sessionID",
        "sessionCleanupProven: true",
        "sessionAuthorityRetained: false",
    ] {
        assert!(
            encoded_failure.contains(required),
            "clean XPC admission failure lost {required}"
        );
    }

    let rust_rejection = rust_begin
        .find("if !begin_admitted")
        .map(|offset| &rust_begin[offset..])
        .expect("Rust must handle rejected signed admissions explicitly");
    assert!(rust_rejection.contains("if session_cleanup_is_proven(&response)"));
    assert!(rust_rejection.contains("release_global_session(session_id)?"));

    let launch = MACOS_XPC_CLIENT_SOURCE
        .find("func schoolx_git_xpc_launch")
        .map(|offset| &MACOS_XPC_CLIENT_SOURCE[offset..])
        .expect("macOS XPC Git launch must remain explicit");
    let launch_end = launch
        .find("\nprivate func installClientOperation")
        .expect("macOS XPC Git launch boundary must remain explicit");
    let launch = &launch[..launch_end];
    assert!(!launch.contains("capabilityDiagnostic()"));
    assert!(launch.contains("lookupClientSession(session_id)"));
    assert!(launch.contains("session.reserveChild(request_id)"));

    let reserve_child = MACOS_XPC_SESSION_SOURCE
        .find("func reserveChild(_ requestID: UInt64) -> Bool")
        .map(|offset| &MACOS_XPC_SESSION_SOURCE[offset..])
        .expect("macOS client child reservation must remain explicit");
    let reserve_child_end = reserve_child
        .find("\n  func ")
        .expect("macOS client child reservation boundary must remain explicit");
    let reserve_child = &reserve_child[..reserve_child_end];
    for required in [
        "admitted",
        "!ending",
        "!quarantined",
        "activeRequestID == 0",
    ] {
        assert!(
            reserve_child.contains(required),
            "macOS client-session reuse gate lost {required}"
        );
    }
    for required in [
        "state.matchesSession(message)",
        "state.reserveLaunch(",
        "state.validateSessionGitReservation()",
    ] {
        assert!(
            MACOS_XPC_SERVICE_SOURCE.contains(required),
            "macOS service launch gate lost {required}"
        );
    }
}

#[test]
fn read_only_code_lists_batch_authenticated_git_and_do_not_poll_it() {
    assert!(PINNED_GIT_SOURCE.contains("pub(crate) fn with_execution_root_authority<T>"));

    let thread_list = CODE_WORKSPACE_COMMAND_SOURCE
        .find("fn list_threads_native")
        .map(|offset| &CODE_WORKSPACE_COMMAND_SOURCE[offset..])
        .expect("bound-thread list must remain native");
    let thread_list_end = thread_list
        .find("\nfn require_binding")
        .expect("bound-thread list boundary must remain explicit");
    let thread_list = &thread_list[..thread_list_end];
    let batch = thread_list
        .find("with_execution_root_authority(||")
        .expect("bound-thread roots must share one authenticated Git session");
    let validation = thread_list
        .find("revalidate_binding_root(&snapshot.binding, nest_root)")
        .expect("each bound root must still be revalidated");
    let authority_end = thread_list
        .find("let mut data")
        .expect("Codex hydration must remain separate from Git authority");
    let runtime_read = thread_list
        .find("runtime.thread_read")
        .expect("validated bindings must still be hydrated from Codex");
    assert!(batch < validation && validation < authority_end && authority_end < runtime_read);

    let inventory = WORKTREE_INVENTORY_SOURCE
        .find("pub fn list_worktree_inventory")
        .map(|offset| &WORKTREE_INVENTORY_SOURCE[offset..])
        .expect("managed-worktree inventory must remain native");
    let inventory_end = inventory
        .find("\nfn inspect_before_deadline")
        .expect("managed-worktree inventory boundary must remain explicit");
    let inventory = &inventory[..inventory_end];
    let batch = inventory
        .find("with_execution_root_authority(||")
        .expect("inventory Git reads must share one authenticated session");
    let deadline = inventory
        .find("let deadline = Instant::now()")
        .expect("inventory reads must remain time-bounded after admission");
    let inspection = inventory
        .find("inspect_before_deadline")
        .expect("inventory roots must still be inspected");
    let merge_proof = inventory
        .find("inventory_merge_proof")
        .expect("archived roots must retain merge proof");
    assert!(batch < deadline && deadline < inspection && inspection < merge_proof);

    assert!(!CODE_WORKSPACE_SCREEN_SOURCE.contains("refetchInterval: runtimeReady ? 5_000 : false"));
    assert!(CODE_WORKSPACE_SCREEN_SOURCE.contains("codeSessionQueryKeys.threads(scope)"));
}

#[test]
fn macos_descriptor_stat_preserves_signed_device_bit_pattern() {
    assert!(MACOS_XPC_SUPPORT_SOURCE.contains("device: UInt64(bitPattern: Int64(value.st_dev))"));
    assert!(!MACOS_XPC_SUPPORT_SOURCE.contains("device: UInt64(value.st_dev)"));
}

#[test]
fn macos_xpc_reaper_ignores_nonterminal_waitid_events() {
    for required in [
        "func consumeInitialSuspendedChildStatus(pid: Int32)",
        "stopped.si_pid == pid, stopped.si_code == CLD_STOPPED",
        "func waitForTerminalChildStatus(pid: Int32)",
        "case CLD_EXITED, CLD_KILLED, CLD_DUMPED:",
        "case CLD_STOPPED, CLD_TRAPPED:",
        "options: WSTOPPED",
        "case CLD_CONTINUED:",
        "options: WCONTINUED",
    ] {
        assert!(
            MACOS_XPC_SUPPORT_SOURCE.contains(required),
            "macOS child reaper contract lost {required}"
        );
    }
    assert!(MACOS_XPC_SUPPORT_SOURCE.contains("options | WNOHANG"));
    let suspended = MACOS_XPC_SERVICE_SOURCE
        .find("consumeInitialSuspendedChildStatus(pid: pid)")
        .expect("macOS helper must confirm the initial suspended child status");
    let reaper = MACOS_XPC_SERVICE_SOURCE
        .rfind("startServiceReaper(")
        .expect("macOS helper must retain a terminal child reaper");
    let started = MACOS_XPC_SERVICE_SOURCE
        .find("xpc_dictionary_set_string(reply, \"kind\", \"started\")")
        .expect("macOS helper must send an exact Started reply");
    assert!(suspended < reaper && reaper < started);
    assert!(MACOS_XPC_SERVICE_SOURCE.contains("waitForTerminalChildStatus(pid: pid)"));
    assert!(
        !MACOS_XPC_SERVICE_SOURCE.contains("waitid(P_PID, id_t(pid), &info, WEXITED | WNOWAIT)")
    );
}

#[test]
fn high_level_git_operations_hold_one_explicit_macos_end_fence() {
    assert!(
        GIT_WRITE_ENGINE_SOURCE
            .matches("super::with_git_authority(||")
            .count()
            >= 4
    );
    assert!(
        GIT_WRITE_RECOVERY_SOURCE
            .matches("super::super::with_git_authority(||")
            .count()
            >= 3
    );
    assert!(GIT_WRITE_STARTUP_SOURCE.contains(
        "super::with_git_authority(|| execute_recovery_plan(plan, recover_removals, recover_git))"
    ));

    for required in [
        "fn with_pinned_git_authority<T>",
        "PinnedGitRequest::ReadOnly",
        "run_pinned_read_until(",
        "prepare_execution_root_with_merge_target_inner",
        "revalidate_execution_root_with_authority",
        "prove_direct_local_ancestry_with_hook_inner",
        "#[cfg(not(unix))]\nfn run_git_until(",
    ] {
        assert!(
            PINNED_GIT_SOURCE.contains(required),
            "pinned Git authority contract lost {required}"
        );
    }
    assert!(REMOVAL_GIT_SOURCE.contains("fn with_removal_git_authority<T>"));
    assert!(REMOVAL_GIT_SOURCE.contains("with_removal_git_authority(|| {"));
    assert!(PROJECT_GIT_EXEC_SOURCE.contains("fn with_pinned_git_directory<T>"));
    assert!(CODE_WORKSPACE_COMMAND_SOURCE.contains("with_pinned_git_directory("));
    assert!(CODE_GIT_HANDOFF_SOURCE.contains("with_pinned_git_directory("));
}

#[test]
fn ambiguous_macos_child_cleanup_never_joins_live_pipe_readers() {
    assert!(GIT_WRITE_SOURCE.contains("mod capture;"));
    assert!(GIT_WRITE_CAPTURE_SOURCE.contains("child.terminate()"));
    for source in [PINNED_GIT_SOURCE, REMOVAL_GIT_SOURCE] {
        assert!(source.contains("child.terminate()"));
    }
    assert!(GIT_WRITE_CAPTURE_SOURCE.contains("drop(stdout_reader)"));
    assert!(GIT_WRITE_CAPTURE_SOURCE.contains("drop(stderr_reader)"));
    assert!(PINNED_GIT_SOURCE.contains("drop(stdout_thread)"));
    assert!(PINNED_GIT_SOURCE.contains("drop(stderr_thread)"));
    assert!(REMOVAL_GIT_SOURCE.contains("drop(stdout)"));
    assert!(REMOVAL_GIT_SOURCE.contains("drop(stderr)"));
}

#[test]
fn every_macos_xpc_source_is_built_and_below_the_size_ceiling() {
    for (path, source) in [
        ("SchoolXGitXpc.swift", MACOS_XPC_CLIENT_SOURCE),
        ("SchoolXGitXpcMessages.swift", MACOS_XPC_MESSAGES_SOURCE),
        ("SchoolXGitXpcService.swift", MACOS_XPC_SERVICE_SOURCE),
        ("SchoolXGitXpcLifecycle.swift", MACOS_XPC_LIFECYCLE_SOURCE),
        ("SchoolXGitXpcSession.swift", MACOS_XPC_SESSION_SOURCE),
        ("SchoolXGitXpcSupport.swift", MACOS_XPC_SUPPORT_SOURCE),
    ] {
        assert!(
            source.lines().count() < 1_000,
            "{path} crossed the 1,000-line source ceiling"
        );
        assert!(
            TAURI_BUILD_SOURCE.contains(path),
            "{path} is not compiled by the Tauri build"
        );
    }
}

#[test]
fn non_unix_worktree_mutation_rejects_before_repository_or_nest_access() {
    let inner = PINNED_GIT_SOURCE
        .find("fn prepare_execution_root_with_merge_target_inner")
        .map(|offset| &PINNED_GIT_SOURCE[offset..])
        .expect("managed-worktree inner preparation must remain explicit");
    let unsupported = inner
        .find("SchoolX Code managed worktree launch is unsupported on this platform")
        .expect("non-Unix managed worktrees must reject deterministically");
    let repository = inner
        .find("let repository = discover_repository")
        .expect("repository discovery must remain explicit");
    assert!(unsupported < repository);
    assert!(!PINNED_GIT_SOURCE.contains("#[cfg(not(unix))]\nfn add_managed_worktree"));
    assert!(!PINNED_GIT_SOURCE.contains("#[cfg(not(unix))]\nfn checkout_managed_worktree"));
}
