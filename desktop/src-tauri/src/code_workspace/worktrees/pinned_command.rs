use super::*;
#[cfg(unix)]
use super::{pinned_operation::*, pinned_verify::*, process::*, repository::*};

#[cfg(all(unix, test))]
pub(super) fn pinned_git_command(request: &PinnedGitRequest) -> Result<Command, String> {
    let git_executable = validate_helper_git_executable(
        request
            .git_executable()
            .to_str()
            .ok_or_else(|| "pinned Git executable was not UTF-8".to_string())?,
    )?;
    let mut command = Command::new(git_executable);
    configure_pinned_git_command(&mut command, request)?;
    Ok(command)
}

#[cfg(unix)]
pub(super) fn configure_pinned_git_command(
    command: &mut Command,
    request: &PinnedGitRequest,
) -> Result<(), String> {
    let (arguments, disabled_filter_keys): (Vec<OsString>, &[String]) = match request {
        PinnedGitRequest::WorktreeAdd {
            git_common_dir,
            base_commit,
            disabled_filter_keys,
            ..
        } => {
            let git_common_dir = canonical_existing_directory(
                Path::new(git_common_dir),
                "pinned Git common directory",
            )?;
            (
                vec![
                    OsString::from(format!(
                        "--git-dir={}",
                        path_to_string(&git_common_dir, "pinned Git common directory")?
                    )),
                    OsString::from("worktree"),
                    OsString::from("add"),
                    OsString::from("--detach"),
                    OsString::from("--no-checkout"),
                    OsString::from("--"),
                    OsString::from("."),
                    OsString::from(base_commit),
                ],
                disabled_filter_keys,
            )
        }
        PinnedGitRequest::Checkout {
            base_commit,
            disabled_filter_keys,
            ..
        } => (
            vec![
                OsString::from("checkout"),
                OsString::from("--detach"),
                OsString::from(base_commit),
            ],
            disabled_filter_keys,
        ),
        PinnedGitRequest::ReadOnly {
            command,
            disabled_filter_keys,
            ..
        } => (pinned_read_arguments(command), disabled_filter_keys),
    };
    command.arg("--no-pager").args(arguments);
    let operation = match request {
        PinnedGitRequest::ReadOnly {
            command:
                CodePinnedReadCommand::StatusPorcelain
                | CodePinnedReadCommand::TrackedNumstat { .. }
                | CodePinnedReadCommand::TrackedNameStatus { .. }
                | CodePinnedReadCommand::TrackedUnmergedPaths
                | CodePinnedReadCommand::TrackedPatch { .. }
                | CodePinnedReadCommand::UntrackedPaths
                | CodePinnedReadCommand::UntrackedPatch { .. },
            ..
        } => GitOperation::WorkingTreeRead,
        PinnedGitRequest::ReadOnly { .. } => GitOperation::ReadOnly,
        PinnedGitRequest::WorktreeAdd { .. } | PinnedGitRequest::Checkout { .. } => {
            GitOperation::Mutating
        }
    };
    configure_git_environment(command, operation, disabled_filter_keys);
    if matches!(
        request,
        PinnedGitRequest::ReadOnly {
            command: CodePinnedReadCommand::UntrackedPatch { .. },
            ..
        }
    ) {
        command.env("LANG", "C").env("LC_ALL", "C");
    }
    command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    crate::util::configure_no_window(command);
    Ok(())
}

#[cfg(unix)]
pub(super) fn pinned_read_arguments(command: &CodePinnedReadCommand) -> Vec<OsString> {
    let literal = OsString::from("--literal-pathspecs");
    match command {
        CodePinnedReadCommand::TopLevel => vec![
            OsString::from("rev-parse"),
            OsString::from("--path-format=absolute"),
            OsString::from("--show-toplevel"),
        ],
        CodePinnedReadCommand::CommonDir => vec![
            OsString::from("rev-parse"),
            OsString::from("--path-format=absolute"),
            OsString::from("--git-common-dir"),
        ],
        CodePinnedReadCommand::LocalConfig => vec![
            OsString::from("config"),
            OsString::from("--local"),
            OsString::from("--includes"),
            OsString::from("--null"),
            OsString::from("--list"),
        ],
        CodePinnedReadCommand::WorktreeConfigNames => vec![
            OsString::from("config"),
            OsString::from("--worktree"),
            OsString::from("--includes"),
            OsString::from("--null"),
            OsString::from("--name-only"),
            OsString::from("--list"),
        ],
        CodePinnedReadCommand::ResolveCommit { base_ref } => vec![
            OsString::from("rev-parse"),
            OsString::from("--verify"),
            OsString::from("--quiet"),
            OsString::from("--end-of-options"),
            OsString::from(format!("{base_ref}^{{commit}}")),
        ],
        CodePinnedReadCommand::VerifyCommit { commit } => vec![
            OsString::from("rev-parse"),
            OsString::from("--verify"),
            OsString::from("--quiet"),
            OsString::from(commit),
        ],
        CodePinnedReadCommand::HeadCommit => vec![
            OsString::from("rev-parse"),
            OsString::from("--verify"),
            OsString::from("--quiet"),
            OsString::from("--end-of-options"),
            OsString::from("HEAD^{commit}"),
        ],
        CodePinnedReadCommand::CurrentBranch => {
            vec![OsString::from("branch"), OsString::from("--show-current")]
        }
        CodePinnedReadCommand::StatusPorcelain => vec![
            OsString::from("status"),
            OsString::from("--porcelain=v1"),
            OsString::from("-z"),
            OsString::from("--untracked-files=normal"),
        ],
        CodePinnedReadCommand::DirectLocalRefCommit { target_ref } => vec![
            OsString::from("rev-parse"),
            OsString::from("--verify"),
            OsString::from("--quiet"),
            OsString::from("--end-of-options"),
            OsString::from(format!("{target_ref}^{{commit}}")),
        ],
        CodePinnedReadCommand::MergeBaseIsAncestor {
            head_commit,
            target_commit,
        } => vec![
            OsString::from("merge-base"),
            OsString::from("--is-ancestor"),
            OsString::from("--end-of-options"),
            OsString::from(head_commit),
            OsString::from(target_commit),
        ],
        CodePinnedReadCommand::TrackedNumstat { base_commit } => vec![
            literal,
            OsString::from("diff"),
            OsString::from("--no-ext-diff"),
            OsString::from("--no-textconv"),
            OsString::from("--no-renames"),
            OsString::from("--numstat"),
            OsString::from("-z"),
            OsString::from(base_commit),
            OsString::from("--"),
        ],
        CodePinnedReadCommand::TrackedNameStatus { base_commit } => vec![
            literal,
            OsString::from("diff"),
            OsString::from("--no-ext-diff"),
            OsString::from("--no-textconv"),
            OsString::from("--no-renames"),
            OsString::from("--name-status"),
            OsString::from("-z"),
            OsString::from(base_commit),
            OsString::from("--"),
        ],
        CodePinnedReadCommand::TrackedUnmergedPaths => vec![
            literal,
            OsString::from("diff"),
            OsString::from("--no-ext-diff"),
            OsString::from("--no-textconv"),
            OsString::from("--no-renames"),
            OsString::from("--name-only"),
            OsString::from("--diff-filter=U"),
            OsString::from("-z"),
            OsString::from("--"),
        ],
        CodePinnedReadCommand::TrackedPatch { base_commit, path } => vec![
            literal,
            OsString::from("diff"),
            OsString::from("--no-ext-diff"),
            OsString::from("--no-textconv"),
            OsString::from("--no-renames"),
            OsString::from("--unified=80"),
            OsString::from("--src-prefix=a/"),
            OsString::from("--dst-prefix=b/"),
            OsString::from(base_commit),
            OsString::from("--"),
            OsString::from(path),
        ],
        CodePinnedReadCommand::UntrackedPaths => vec![
            literal,
            OsString::from("ls-files"),
            OsString::from("--others"),
            OsString::from("--exclude-standard"),
            OsString::from("-z"),
            OsString::from("--"),
        ],
        CodePinnedReadCommand::UntrackedPatch { path: _ } => vec![
            literal,
            OsString::from("diff"),
            OsString::from("--no-index"),
            OsString::from("--no-ext-diff"),
            OsString::from("--no-textconv"),
            OsString::from("--patch"),
            OsString::from("--unified=80"),
            OsString::from("--src-prefix=a/"),
            OsString::from("--dst-prefix=b/"),
            OsString::from("--"),
            OsString::from("/dev/null"),
            OsString::from("/dev/fd/0"),
        ],
    }
}

#[cfg(unix)]
pub(super) fn spawn_pinned_git_helper(
    request: &PinnedGitRequest,
    directories: &[fs::File],
    launch: &PinnedGitLaunchAuthority,
) -> Result<Vec<u8>, String> {
    let target = directories
        .last()
        .ok_or_else(|| "pinned Git operation did not carry its target handle".to_string())?;
    let child = launch.spawn(request, target)?;
    #[cfg(all(target_os = "macos", not(test)))]
    return capture_macos_pinned_child(child, "pinned git", GIT_TIMEOUT);
    #[cfg(any(not(target_os = "macos"), test))]
    capture_child(child, "pinned git", GIT_TIMEOUT)
}

#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn prepare_pinned_read_operation(
    execution_root: &Path,
    command: CodePinnedReadCommand,
    disabled_filter_keys: Vec<String>,
) -> Result<CodePinnedGitOperation, String> {
    let request = PinnedGitRequest::ReadOnly {
        // Admission binds this placeholder to the platform's root-trusted Git
        // executable before the envelope is validated or launched.
        git_executable: String::new(),
        command,
        disabled_filter_keys,
        expected_target_path: path_to_string(execution_root, "pinned read-only Git root")?,
    };
    prepare_pinned_git_operation(execution_root, request)
}

#[cfg(unix)]
pub(super) fn run_pinned_read_until(
    execution_root: &Path,
    command: CodePinnedReadCommand,
    disabled_filter_keys: Vec<String>,
    deadline: Option<Instant>,
) -> Result<Vec<u8>, String> {
    remaining_git_timeout(deadline)?;
    let operation = prepare_pinned_read_operation(execution_root, command, disabled_filter_keys)?;
    verify_pinned_target_chain(&operation.request, &operation.directories)?;
    let timeout = remaining_git_timeout(deadline)?;
    let target = operation
        .directories
        .last()
        .ok_or_else(|| "pinned Git operation did not carry its target handle".to_string())?;
    let child = operation.launch.spawn(&operation.request, target)?;
    #[cfg(all(target_os = "macos", not(test)))]
    let result = capture_macos_pinned_child(child, "pinned read-only git", timeout);
    #[cfg(any(not(target_os = "macos"), test))]
    let result = capture_child(child, "pinned read-only git", timeout);
    let verified = verify_pinned_target_chain(&operation.request, &operation.directories);
    match (result, verified) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(output), Ok(())) => Ok(strip_pinned_test_harness_output(output)),
    }
}

#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn run_pinned_read_before(
    execution_root: &Path,
    command: CodePinnedReadCommand,
    deadline: Instant,
) -> Result<Vec<u8>, String> {
    run_pinned_read_until(execution_root, command, Vec::new(), Some(deadline))
}

#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn run_pinned_ancestry_before(
    execution_root: &Path,
    head_commit: &str,
    target_commit: &str,
    deadline: Instant,
) -> Result<bool, String> {
    remaining_git_timeout(Some(deadline))?;
    let operation = prepare_pinned_read_operation(
        execution_root,
        CodePinnedReadCommand::MergeBaseIsAncestor {
            head_commit: head_commit.to_string(),
            target_commit: target_commit.to_string(),
        },
        Vec::new(),
    )?;
    verify_pinned_target_chain(&operation.request, &operation.directories)?;
    let timeout = remaining_git_timeout(Some(deadline))?;
    let target = operation
        .directories
        .last()
        .ok_or_else(|| "pinned Git operation did not carry its target handle".to_string())?;
    let mut child = operation.launch.spawn(&operation.request, target)?;
    #[cfg(all(target_os = "macos", not(test)))]
    let captured = capture_macos_pinned_child_status(&mut child, "pinned merge-base git", timeout);
    #[cfg(any(not(target_os = "macos"), test))]
    let captured = capture_child_status(&mut child, "pinned merge-base git", timeout);
    let verified = verify_pinned_target_chain(&operation.request, &operation.directories);
    let mut captured = match (captured, verified) {
        (Err(error), _) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
        (Ok(captured), Ok(())) => captured,
    };
    captured.stderr.bytes = strip_linux_onnxruntime_startup_diagnostic(captured.stderr.bytes);
    if captured.stdout.truncated || captured.stderr.truncated {
        return Err("pinned merge-base output exceeded the SchoolX Code limit".to_string());
    }
    let stdout = strip_pinned_test_harness_output(captured.stdout.bytes.clone());
    match captured.status.code() {
        Some(0) if stdout.is_empty() && captured.stderr.bytes.is_empty() => Ok(true),
        Some(1) if stdout.is_empty() && captured.stderr.bytes.is_empty() => Ok(false),
        _ => Err(captured_child_error("pinned merge-base git", &captured)),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn strip_pinned_test_harness_output(output: Vec<u8>) -> Vec<u8> {
    #[cfg(test)]
    {
        output
            .strip_prefix(b"\nrunning 1 test\n")
            .unwrap_or(&output)
            .to_vec()
    }
    #[cfg(not(test))]
    output
}

#[cfg(unix)]
pub(crate) fn strip_linux_onnxruntime_startup_diagnostic(output: Vec<u8>) -> Vec<u8> {
    #[cfg(target_os = "linux")]
    {
        // The statically linked ONNX runtime can emit this before `main` on
        // virtualized ARM Linux. Remove only one exact, complete first line;
        // helper/Git stderr after it remains fail-closed.
        const PREFIX: &[u8] =
            b"onnxruntime cpuid_info warning: Unknown CPU vendor. cpuinfo_vendor value: ";
        let Some(after_prefix) = output.strip_prefix(PREFIX) else {
            return output;
        };
        let line_end = after_prefix
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap_or(after_prefix.len());
        let vendor_line = &after_prefix[..line_end];
        let vendor = vendor_line.strip_suffix(b"\r").unwrap_or(vendor_line);
        if vendor.is_empty() || !vendor.iter().all(u8::is_ascii_digit) {
            return output;
        }
        if line_end == after_prefix.len() {
            Vec::new()
        } else {
            after_prefix[line_end + 1..].to_vec()
        }
    }
    #[cfg(not(target_os = "linux"))]
    output
}

/// Spawn one strictly typed read-only Git command with `target` installed as
/// the helper's cwd via `fchdir`. The caller owns output/deadline enforcement.
#[cfg(target_os = "linux")]
pub(crate) fn spawn_pinned_read_git_helper(
    target: &fs::File,
    expected_target_path: &Path,
    _git_executable: &Path,
    launch: &GitLaunchAuthority,
    command: CodePinnedReadCommand,
    disabled_filter_keys: Vec<String>,
) -> Result<Child, String> {
    let request = PinnedGitRequest::ReadOnly {
        git_executable: path_to_string(launch.path(), "pinned read-only Git executable")?,
        command,
        disabled_filter_keys,
        expected_target_path: path_to_string(expected_target_path, "pinned read-only Git root")?,
    };
    validate_pinned_git_envelope(&PinnedGitEnvelope {
        version: PINNED_GIT_REQUEST_VERSION,
        target_device: 0,
        target_inode: 0,
        request: clone_pinned_git_request(&request),
    })?;
    spawn_pinned_git_direct_child(&request, target, launch)
}

#[cfg(all(target_os = "macos", not(test)))]
pub(crate) fn spawn_pinned_read_git_helper(
    target: &fs::File,
    expected_target_path: &Path,
    _git_executable: &Path,
    session: &MacGitAuthoritySession,
    command: CodePinnedReadCommand,
    disabled_filter_keys: Vec<String>,
) -> Result<PinnedGitChild, String> {
    let request = PinnedGitRequest::ReadOnly {
        git_executable: MACOS_SYSTEM_GIT.to_string(),
        command,
        disabled_filter_keys,
        expected_target_path: path_to_string(expected_target_path, "pinned read-only Git root")?,
    };
    validate_pinned_git_envelope(&PinnedGitEnvelope {
        version: PINNED_GIT_REQUEST_VERSION,
        target_device: 0,
        target_inode: 0,
        request: clone_pinned_git_request(&request),
    })?;
    spawn_pinned_git_macos_child(&request, target, session)
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos")), not(test)))]
pub(crate) fn spawn_pinned_read_git_helper(
    _target: &fs::File,
    _expected_target_path: &Path,
    _git_executable: &Path,
    _command: CodePinnedReadCommand,
    _disabled_filter_keys: Vec<String>,
) -> Result<PinnedGitChild, String> {
    Err("pinned Git launch is unsupported on this Unix platform".to_string())
}

#[cfg(all(target_os = "macos", test))]
pub(crate) fn spawn_pinned_read_git_helper(
    target: &fs::File,
    expected_target_path: &Path,
    git_executable: &Path,
    command: CodePinnedReadCommand,
    disabled_filter_keys: Vec<String>,
) -> Result<PinnedGitChild, String> {
    let request = PinnedGitRequest::ReadOnly {
        git_executable: path_to_string(git_executable, "pinned read-only Git executable")?,
        command,
        disabled_filter_keys,
        expected_target_path: path_to_string(expected_target_path, "pinned read-only Git root")?,
    };
    spawn_pinned_git_path_helper_child(&request, target)
}

#[cfg(target_os = "linux")]
pub(super) fn spawn_pinned_git_direct_child(
    request: &PinnedGitRequest,
    target: &fs::File,
    authority: &GitLaunchAuthority,
) -> Result<Child, String> {
    use std::os::fd::AsFd as _;
    use std::os::unix::fs::MetadataExt as _;

    let metadata = target
        .metadata()
        .map_err(|error| format!("failed to inspect pinned Git target: {error}"))?;
    let envelope = PinnedGitEnvelope {
        version: PINNED_GIT_REQUEST_VERSION,
        target_device: metadata.dev(),
        target_inode: metadata.ino(),
        request: clone_pinned_git_request(request),
    };
    validate_pinned_git_envelope(&envelope)?;
    let encoded = serde_json::to_vec(&envelope)
        .map_err(|error| format!("failed to encode pinned Git request: {error}"))?;
    if encoded.len() > MAX_PINNED_GIT_REQUEST_BYTES {
        return Err(format!(
            "pinned Git request exceeded {MAX_PINNED_GIT_REQUEST_BYTES} bytes"
        ));
    }
    if request.git_executable() != authority.path() {
        return Err(
            "pinned Git request did not match its root-trusted launch authority".to_string(),
        );
    }
    let pinned_untracked_file = match request {
        PinnedGitRequest::ReadOnly {
            command: CodePinnedReadCommand::UntrackedPatch { path },
            ..
        } => Some(open_pinned_untracked_file(target.as_fd(), path)?),
        _ => None,
    };
    let mut command = authority.command();
    configure_pinned_git_command(command.command_mut(), request)?;
    authority.spawn(
        target,
        command,
        pinned_untracked_file
            .map(Stdio::from)
            .unwrap_or_else(Stdio::null),
    )
}

#[cfg(all(target_os = "macos", not(test)))]
pub(super) fn spawn_pinned_git_macos_child(
    request: &PinnedGitRequest,
    target: &fs::File,
    session: &MacGitAuthoritySession,
) -> Result<MacGitChild, String> {
    use std::os::fd::AsFd as _;
    use std::os::unix::fs::MetadataExt as _;

    let metadata = target
        .metadata()
        .map_err(|error| format!("failed to inspect pinned Git target: {error}"))?;
    let envelope = PinnedGitEnvelope {
        version: PINNED_GIT_REQUEST_VERSION,
        target_device: metadata.dev(),
        target_inode: metadata.ino(),
        request: clone_pinned_git_request(request),
    };
    validate_pinned_git_envelope(&envelope)?;
    if request.git_executable() != Path::new(MACOS_SYSTEM_GIT) {
        return Err(
            "pinned Git request did not match the macOS root-trusted launch authority".to_string(),
        );
    }
    let encoded = serde_json::to_string(&envelope)
        .map_err(|error| format!("failed to encode pinned Git request: {error}"))?;
    if encoded.len() > MAX_PINNED_GIT_REQUEST_BYTES {
        return Err(format!(
            "pinned Git request exceeded {MAX_PINNED_GIT_REQUEST_BYTES} bytes"
        ));
    }
    let input = match request {
        PinnedGitRequest::ReadOnly {
            command: CodePinnedReadCommand::UntrackedPatch { path },
            ..
        } => MacGitInput::File(open_pinned_untracked_file(target.as_fd(), path)?),
        _ => MacGitInput::Null,
    };
    session.spawn(MacGitFamily::Pinned, encoded, target, input)
}

#[cfg(all(unix, not(target_os = "linux"), test))]
pub(super) fn spawn_pinned_git_path_helper_child(
    request: &PinnedGitRequest,
    target: &fs::File,
) -> Result<Child, String> {
    use std::os::unix::fs::MetadataExt;

    let metadata = target
        .metadata()
        .map_err(|error| format!("failed to inspect pinned Git target: {error}"))?;
    let envelope = PinnedGitEnvelope {
        version: PINNED_GIT_REQUEST_VERSION,
        target_device: metadata.dev(),
        target_inode: metadata.ino(),
        request: clone_pinned_git_request(request),
    };
    let encoded = serde_json::to_string(&envelope)
        .map_err(|error| format!("failed to encode pinned Git request: {error}"))?;
    if encoded.len() > MAX_PINNED_GIT_REQUEST_BYTES {
        return Err(format!(
            "pinned Git request exceeded {MAX_PINNED_GIT_REQUEST_BYTES} bytes"
        ));
    }
    let executable = std::env::current_exe()
        .map_err(|error| format!("failed to resolve SchoolX desktop executable: {error}"))?;
    let mut command = Command::new(executable);
    command.args([
        "--exact",
        "code_workspace::worktrees::tests::pinned_git_helper_subprocess_entry",
        "--ignored",
        "--nocapture",
    ]);
    command
        .env(PINNED_GIT_REQUEST_ENV, encoded)
        .stdin(Stdio::from(target.try_clone().map_err(|error| {
            format!("failed to clone pinned Git target: {error}")
        })?))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::util::configure_no_window(&mut command);
    use std::os::unix::process::CommandExt;
    command.process_group(0);
    let child = command
        .spawn()
        .map_err(|error| format!("failed to start pinned Git helper: {error}"))?;
    Ok(child)
}
