use super::*;

#[test]
fn ambiguous_macos_capture_cleanup_detaches_live_pipe_readers() -> Result<(), String> {
    let (stdout_reader, stdout_writer) = std::io::pipe().map_err(|error| error.to_string())?;
    let (stderr_reader, stderr_writer) = std::io::pipe().map_err(|error| error.to_string())?;
    let stdout_thread = spawn_pipe_reader(Some(stdout_reader));
    let stderr_thread = spawn_pipe_reader(Some(stderr_reader));
    let started = Instant::now();
    let error = finish_failed_capture_threads(
        stdout_thread,
        stderr_thread,
        "poll failed".to_string(),
        || Err("cleanup disposition ambiguous".to_string()),
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(error, "poll failed; cleanup disposition ambiguous");
    drop(stdout_writer);
    drop(stderr_writer);
    Ok(())
}

#[test]
fn legacy_path_git_launcher_is_test_or_non_unix_only() {
    let path_launcher_source = include_str!("../git.rs");
    assert!(path_launcher_source.contains("#[cfg(not(unix))]\npub(super) fn run_git_until("));
    assert!(path_launcher_source
        .contains("#[cfg(not(unix))]\npub(super) fn run_git_with_filter_overrides_until("));
    assert!(path_launcher_source
        .contains("#[cfg(any(not(unix), test))]\npub(super) fn git_executable("));

    let executable_launcher_source = include_str!("../pinned_verify.rs");
    assert!(executable_launcher_source
        .contains("#[cfg(not(unix))]\npub(super) fn run_git_executable_with_filter_overrides("));
}

#[cfg(unix)]
#[test]
fn read_only_pin_uses_one_exact_named_target_descriptor() -> Result<(), String> {
    let sandbox = tempfile::tempdir().map_err(|error| error.to_string())?;
    let target = sandbox.path().join("selected-repository");
    fs::create_dir(&target).map_err(|error| error.to_string())?;
    let target = target.canonicalize().map_err(|error| error.to_string())?;
    let request = PinnedGitRequest::ReadOnly {
        git_executable: path_to_string(&git_executable()?, "test Git executable")?,
        command: CodePinnedReadCommand::TopLevel,
        disabled_filter_keys: Vec::new(),
        expected_target_path: path_to_string(&target, "test read-only target")?,
    };
    let directories = pin_pinned_git_directories(&target, &request)?;
    assert_eq!(directories.len(), 1);
    verify_pinned_target_chain(&request, &directories)?;

    let moved = sandbox.path().join("moved-selected-repository");
    fs::rename(&target, &moved).map_err(|error| error.to_string())?;
    fs::create_dir(&target).map_err(|error| error.to_string())?;
    assert!(verify_pinned_target_chain(&request, &directories)
        .is_err_and(|error| error.contains("moved or was replaced")));
    Ok(())
}

#[cfg(target_os = "macos")]
#[test]
fn macos_xpc_prepare_revalidates_pinned_git_descriptors() -> Result<(), String> {
    use std::os::unix::fs::MetadataExt as _;

    if rustix::process::geteuid().as_raw() == 0 {
        return Ok(());
    }
    let target = tempfile::tempdir().map_err(|error| error.to_string())?;
    let metadata = target
        .path()
        .metadata()
        .map_err(|error| error.to_string())?;
    let envelope = PinnedGitEnvelope {
        version: PINNED_GIT_REQUEST_VERSION,
        target_device: metadata.dev(),
        target_inode: metadata.ino(),
        request: PinnedGitRequest::ReadOnly {
            git_executable: MACOS_SYSTEM_GIT.to_string(),
            command: CodePinnedReadCommand::TopLevel,
            disabled_filter_keys: Vec::new(),
            expected_target_path: path_to_string(target.path(), "test target")?,
        },
    };
    let payload = serde_json::to_string(&envelope).map_err(|error| error.to_string())?;
    let cwd = DescriptorObservation {
        device: metadata.dev(),
        inode: metadata.ino(),
        mode: metadata.mode(),
        size: 0,
    };
    let null = fs::metadata("/dev/null").map_err(|error| error.to_string())?;
    let stdin = DescriptorObservation {
        device: null.dev(),
        inode: null.ino(),
        mode: null.mode(),
        size: null.size(),
    };
    prepare_macos_pinned_git(&payload, cwd, stdin)?;
    assert!(prepare_macos_pinned_git(
        &payload,
        DescriptorObservation {
            inode: cwd.inode.saturating_add(1),
            ..cwd
        },
        stdin,
    )
    .is_err_and(|error| error.contains("descriptor identity")));
    Ok(())
}

#[test]
fn pinned_change_inventory_commands_are_literal_closed_and_non_renaming() {
    let base_commit = "a".repeat(40);
    let name_status = pinned_read_arguments(&CodePinnedReadCommand::TrackedNameStatus {
        base_commit: base_commit.clone(),
    });
    let name_status = name_status
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        name_status,
        vec![
            "--literal-pathspecs",
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--name-status",
            "-z",
            &base_commit,
            "--",
        ]
    );

    let unmerged = pinned_read_arguments(&CodePinnedReadCommand::TrackedUnmergedPaths)
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(
        unmerged,
        vec![
            "--literal-pathspecs",
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--name-only",
            "--diff-filter=U",
            "-z",
            "--",
        ]
    );

    let untracked = pinned_read_arguments(&CodePinnedReadCommand::UntrackedPatch {
        path: "untracked.txt".to_string(),
    });
    let untracked = untracked
        .iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(untracked.iter().any(|argument| argument == "--patch"));
    assert!(!untracked.iter().any(|argument| argument == "--binary"));
    assert!(!untracked.iter().any(|argument| argument == "--numstat"));
}

#[cfg(unix)]
#[test]
fn merge_proof_commands_and_environment_are_literal_and_closed() {
    let head_commit = "a".repeat(40);
    let target_commit = "b".repeat(40);
    let merge_base = pinned_read_arguments(&CodePinnedReadCommand::MergeBaseIsAncestor {
        head_commit: head_commit.clone(),
        target_commit: target_commit.clone(),
    })
    .into_iter()
    .map(|value| value.to_string_lossy().into_owned())
    .collect::<Vec<_>>();
    assert_eq!(
        merge_base,
        vec![
            "merge-base",
            "--is-ancestor",
            "--end-of-options",
            &head_commit,
            &target_commit,
        ]
    );

    let target_ref = "refs/heads/main".to_string();
    let direct_ref = pinned_read_arguments(&CodePinnedReadCommand::DirectLocalRefCommit {
        target_ref: target_ref.clone(),
    })
    .into_iter()
    .map(|value| value.to_string_lossy().into_owned())
    .collect::<Vec<_>>();
    assert_eq!(
        direct_ref,
        vec![
            "rev-parse",
            "--verify",
            "--quiet",
            "--end-of-options",
            &format!("{target_ref}^{{commit}}"),
        ]
    );

    assert!(
        validate_pinned_read_command(&CodePinnedReadCommand::DirectLocalRefCommit {
            target_ref: "refs/remotes/origin/main".to_string(),
        })
        .is_err()
    );
    assert!(
        validate_pinned_read_command(&CodePinnedReadCommand::MergeBaseIsAncestor {
            head_commit: "HEAD".to_string(),
            target_commit,
        })
        .is_err()
    );

    let mut command = Command::new("git");
    configure_git_environment(&mut command, GitOperation::ReadOnly, &[]);
    let environment = command
        .get_envs()
        .filter_map(|(key, value)| {
            value.map(|value| {
                (
                    key.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        environment
            .get("GIT_NO_REPLACE_OBJECTS")
            .map(String::as_str),
        Some("1")
    );
    assert_eq!(
        environment.get("GIT_NO_LAZY_FETCH").map(String::as_str),
        Some("1")
    );
    assert_eq!(
        environment.get("GIT_GRAFT_FILE").map(String::as_str),
        Some("/dev/null")
    );
    assert_eq!(
        environment.get("GIT_OPTIONAL_LOCKS").map(String::as_str),
        Some("0")
    );
    let configured_keys = environment
        .iter()
        .filter_map(|(key, value)| key.starts_with("GIT_CONFIG_KEY_").then_some(value.as_str()))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        configured_keys,
        [
            "advice.graftFileDeprecated",
            "core.fsmonitor",
            "core.hooksPath",
            "credential.helper",
            "protocol.allow",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>()
    );
}
