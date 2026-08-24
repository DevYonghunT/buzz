use super::{pinned_command::*, pinned_verify::*, repository::*, *};

#[cfg(unix)]
pub(super) fn run_git_in_pinned_directory(
    directory_path: &Path,
    request: PinnedGitRequest,
    launch: &PinnedGitLaunchAuthority,
) -> Result<Vec<u8>, String> {
    let operation = prepare_pinned_git_operation_with_launch(directory_path, request, launch)?;
    run_git_with_pinned_operation(operation)
}

#[cfg(unix)]
pub(super) fn prepare_pinned_git_operation(
    directory_path: &Path,
    mut request: PinnedGitRequest,
) -> Result<CodePinnedGitOperation, String> {
    let directories = pin_pinned_git_directories(directory_path, &request)?;
    let target = directories
        .last()
        .ok_or_else(|| "pinned Git operation did not carry its target handle".to_string())?;
    let launch = PinnedGitLaunchAuthority::admit(target)?;
    launch.bind_request(&mut request)?;
    verify_pinned_target_chain(&request, &directories)?;
    Ok(CodePinnedGitOperation {
        request,
        directories,
        launch,
    })
}

#[cfg(unix)]
pub(super) fn prepare_pinned_git_operation_with_launch(
    directory_path: &Path,
    mut request: PinnedGitRequest,
    launch: &PinnedGitLaunchAuthority,
) -> Result<CodePinnedGitOperation, String> {
    let directories = pin_pinned_git_directories(directory_path, &request)?;
    launch.bind_request(&mut request)?;
    verify_pinned_target_chain(&request, &directories)?;
    Ok(CodePinnedGitOperation {
        request,
        directories,
        launch: launch.clone(),
    })
}

#[cfg(unix)]
pub(super) fn pin_pinned_git_directories(
    directory_path: &Path,
    request: &PinnedGitRequest,
) -> Result<Vec<fs::File>, String> {
    let expected_target = request.expected_target_path();
    if expected_target != directory_path {
        return Err("pinned Git target did not match its native request".to_string());
    }
    let target = pin_git_directory(directory_path)?;
    if matches!(request, PinnedGitRequest::ReadOnly { .. }) {
        let directories = vec![target];
        verify_pinned_target_chain(request, &directories)?;
        return Ok(directories);
    }
    let repository_bucket = directory_path
        .parent()
        .ok_or_else(|| "managed Git target had no repository bucket".to_string())?;
    let worktrees_root = repository_bucket
        .parent()
        .ok_or_else(|| "managed Git target had no WORKTREES root".to_string())?;
    let nest_root = worktrees_root
        .parent()
        .ok_or_else(|| "managed Git target had no nest root".to_string())?;
    let directories = vec![
        pin_git_directory(nest_root)?,
        pin_git_directory(worktrees_root)?,
        pin_git_directory(repository_bucket)?,
        target,
    ];
    verify_pinned_target_chain(request, &directories)?;
    Ok(directories)
}

#[cfg(unix)]
pub(super) fn pin_git_directory(directory_path: &Path) -> Result<fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;

    let directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory_path)
        .map_err(|error| {
            format!(
                "failed to pin SchoolX Code Git directory {}: {error}",
                directory_path.display()
            )
        })?;
    if !directory
        .metadata()
        .map_err(|error| format!("failed to inspect pinned Git directory: {error}"))?
        .is_dir()
    {
        return Err("pinned SchoolX Code Git target is not a directory".to_string());
    }
    Ok(directory)
}

#[cfg(unix)]
pub(super) fn run_git_with_pinned_operation(
    operation: CodePinnedGitOperation,
) -> Result<Vec<u8>, String> {
    operation.execute()
}

/// Test-only subprocess entry that preserves crash/race regression coverage
/// without participating in production launch authority.
#[cfg(all(unix, test))]
pub(super) fn execute_pinned_git_helper() -> Result<(), String> {
    use std::os::fd::AsFd;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::process::CommandExt;

    let encoded = std::env::var(PINNED_GIT_REQUEST_ENV)
        .map_err(|_| "pinned Git helper request was missing or not UTF-8".to_string())?;
    if encoded.len() > MAX_PINNED_GIT_REQUEST_BYTES {
        return Err(format!(
            "pinned Git helper request exceeded {MAX_PINNED_GIT_REQUEST_BYTES} bytes"
        ));
    }
    let envelope: PinnedGitEnvelope = serde_json::from_str(&encoded)
        .map_err(|error| format!("invalid pinned Git helper request: {error}"))?;
    validate_pinned_git_envelope(&envelope)?;

    let stdin = std::io::stdin();
    let stat = rustix::fs::fstat(stdin.as_fd())
        .map_err(|error| format!("failed to inspect pinned Git helper directory: {error}"))?;
    if !rustix::fs::FileType::from_raw_mode(stat.st_mode).is_dir()
        || stat.st_dev as u64 != envelope.target_device
        || stat.st_ino as u64 != envelope.target_inode
    {
        return Err("pinned Git helper directory identity did not match its request".to_string());
    }
    rustix::process::fchdir(stdin.as_fd())
        .map_err(|error| format!("failed to enter pinned Git directory: {error}"))?;
    let current = fs::metadata(".")
        .map_err(|error| format!("failed to verify pinned Git working directory: {error}"))?;
    if current.dev() != envelope.target_device || current.ino() != envelope.target_inode {
        return Err("pinned Git helper changed to a different directory".to_string());
    }

    let pinned_untracked_file = match &envelope.request {
        PinnedGitRequest::ReadOnly {
            command: CodePinnedReadCommand::UntrackedPatch { path },
            ..
        } => Some(open_pinned_untracked_file(stdin.as_fd(), path)?),
        _ => None,
    };
    let mut command = pinned_git_command(&envelope.request)?;
    if let Some(file) = pinned_untracked_file {
        command.stdin(Stdio::from(file));
    } else {
        command.stdin(Stdio::null());
    }
    let error = command.exec();
    Err(format!("failed to execute pinned Git: {error}"))
}

#[cfg(unix)]
pub(super) fn open_pinned_untracked_file(
    root: std::os::fd::BorrowedFd<'_>,
    relative: &str,
) -> Result<fs::File, String> {
    use std::os::fd::AsFd;
    use std::path::Component;

    let components = Path::new(relative)
        .components()
        .map(|component| match component {
            Component::Normal(value) => Ok(value),
            _ => Err("pinned untracked path was not repository-relative".to_string()),
        })
        .collect::<Result<Vec<_>, String>>()?;
    let (file_name, ancestors) = components
        .split_last()
        .ok_or_else(|| "pinned untracked path was empty".to_string())?;
    let mut directories = Vec::with_capacity(ancestors.len());
    for component in ancestors {
        let parent = directories
            .last()
            .map_or(root, |directory: &rustix::fd::OwnedFd| directory.as_fd());
        let directory = rustix::fs::openat(
            parent,
            *component,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|error| format!("failed to pin untracked path ancestor: {error}"))?;
        directories.push(directory);
    }
    let parent = directories
        .last()
        .map_or(root, |directory| directory.as_fd());
    let file = rustix::fs::openat(
        parent,
        *file_name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| format!("failed to pin untracked file: {error}"))?;
    let stat = rustix::fs::fstat(&file)
        .map_err(|error| format!("failed to inspect pinned untracked file: {error}"))?;
    if !rustix::fs::FileType::from_raw_mode(stat.st_mode).is_file() {
        return Err("pinned untracked entry is not a regular file".to_string());
    }
    Ok(fs::File::from(file))
}

#[cfg(unix)]
pub(super) fn validate_pinned_git_envelope(envelope: &PinnedGitEnvelope) -> Result<(), String> {
    if envelope.version != PINNED_GIT_REQUEST_VERSION {
        return Err(format!(
            "unsupported pinned Git request version {}",
            envelope.version
        ));
    }
    let expected_target = envelope.request.expected_target_path();
    if !expected_target.is_absolute() {
        return Err("pinned Git target path must be absolute".to_string());
    }
    if expected_target.to_string_lossy().len() > MAX_PINNED_GIT_PATH_BYTES {
        return Err("pinned Git target path exceeded its safety limit".to_string());
    }
    match &envelope.request {
        PinnedGitRequest::WorktreeAdd {
            git_executable,
            git_common_dir,
            base_commit,
            disabled_filter_keys,
            ..
        } => {
            validate_helper_path_length(git_executable)?;
            validate_helper_path_length(git_common_dir)?;
            canonical_existing_directory(Path::new(git_common_dir), "pinned Git common directory")?;
            validate_commit_id(base_commit)?;
            validate_filter_override_keys(disabled_filter_keys)
        }
        PinnedGitRequest::Checkout {
            git_executable,
            base_commit,
            disabled_filter_keys,
            ..
        } => {
            validate_helper_path_length(git_executable)?;
            validate_commit_id(base_commit)?;
            validate_filter_override_keys(disabled_filter_keys)
        }
        PinnedGitRequest::ReadOnly {
            git_executable,
            command,
            disabled_filter_keys,
            ..
        } => {
            validate_helper_path_length(git_executable)?;
            validate_pinned_read_command(command)?;
            validate_filter_override_keys(disabled_filter_keys)
        }
    }
}

/// Decode and revalidate one closed pinned-Git envelope inside the signed
/// macOS service, then derive its fixed `/usr/bin/git` process specification.
#[cfg(target_os = "macos")]
pub(in crate::code_workspace) fn prepare_macos_pinned_git(
    payload: &str,
    cwd: DescriptorObservation,
    stdin: DescriptorObservation,
) -> Result<MacGitProcessSpec, String> {
    if payload.len() > MAX_PINNED_GIT_REQUEST_BYTES {
        return Err(format!(
            "pinned Git request exceeded {MAX_PINNED_GIT_REQUEST_BYTES} bytes"
        ));
    }
    let envelope: PinnedGitEnvelope = serde_json::from_str(payload)
        .map_err(|error| format!("invalid pinned Git helper request: {error}"))?;
    validate_pinned_git_envelope(&envelope)?;
    macos_git_xpc::validate_directory_observation(
        cwd,
        envelope.target_device,
        envelope.target_inode,
        None,
        "pinned Git cwd",
    )?;
    if envelope.request.git_executable() != Path::new(MACOS_SYSTEM_GIT) {
        return Err("macOS pinned Git request did not select /usr/bin/git".to_string());
    }
    let trusted_git = super::super::git_write::macos_root_trusted_git()?;
    match &envelope.request {
        PinnedGitRequest::ReadOnly {
            command: CodePinnedReadCommand::UntrackedPatch { .. },
            ..
        } => macos_git_xpc::validate_bounded_regular_observation(
            stdin,
            u64::MAX,
            "pinned untracked input",
        )?,
        _ => macos_git_xpc::validate_null_observation(stdin, "pinned Git input")?,
    }
    let mut command = Command::new(trusted_git);
    configure_pinned_git_command(&mut command, &envelope.request)?;
    macos_git_xpc::process_spec_from_command(&command)
}

#[cfg(unix)]
pub(super) fn validate_pinned_read_command(command: &CodePinnedReadCommand) -> Result<(), String> {
    match command {
        CodePinnedReadCommand::TopLevel
        | CodePinnedReadCommand::CommonDir
        | CodePinnedReadCommand::LocalConfig
        | CodePinnedReadCommand::WorktreeConfigNames
        | CodePinnedReadCommand::HeadCommit
        | CodePinnedReadCommand::CurrentBranch
        | CodePinnedReadCommand::StatusPorcelain
        | CodePinnedReadCommand::TrackedUnmergedPaths => Ok(()),
        CodePinnedReadCommand::ResolveCommit { base_ref } => {
            validate_base_ref(base_ref).map(|_| ())
        }
        CodePinnedReadCommand::DirectLocalRefCommit { target_ref } => {
            validate_direct_local_branch_ref(target_ref)
        }
        CodePinnedReadCommand::MergeBaseIsAncestor {
            head_commit,
            target_commit,
        } => {
            validate_commit_id(head_commit)?;
            validate_commit_id(target_commit)
        }
        CodePinnedReadCommand::VerifyCommit { commit }
        | CodePinnedReadCommand::TrackedNumstat {
            base_commit: commit,
        }
        | CodePinnedReadCommand::TrackedNameStatus {
            base_commit: commit,
        } => validate_commit_id(commit),
        CodePinnedReadCommand::TrackedPatch { base_commit, path } => {
            validate_commit_id(base_commit)?;
            validate_pinned_read_path(path)
        }
        CodePinnedReadCommand::UntrackedPaths => Ok(()),
        CodePinnedReadCommand::UntrackedPatch { path } => validate_pinned_read_path(path),
    }
}

#[cfg(unix)]
pub(super) fn validate_pinned_read_path(value: &str) -> Result<(), String> {
    use std::path::Component;

    if value.is_empty()
        || value.len() > MAX_PINNED_GIT_PATH_BYTES
        || value.chars().any(char::is_control)
        || !Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err("pinned read-only Git path was invalid".to_string());
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn validate_helper_path_length(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_PINNED_GIT_PATH_BYTES {
        Err("pinned Git path exceeded its safety limit".to_string())
    } else {
        Ok(())
    }
}
