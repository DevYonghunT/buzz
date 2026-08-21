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
    assert!(MACOS_XPC_SOURCE.contains("require_capability()"));
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
