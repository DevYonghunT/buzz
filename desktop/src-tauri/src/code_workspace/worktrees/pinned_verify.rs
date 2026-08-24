#[cfg(not(unix))]
use super::process::*;
use super::{repository::*, *};

#[cfg(unix)]
pub(super) fn clone_pinned_git_request(request: &PinnedGitRequest) -> PinnedGitRequest {
    match request {
        PinnedGitRequest::WorktreeAdd {
            git_executable,
            git_common_dir,
            base_commit,
            disabled_filter_keys,
            expected_target_path,
        } => PinnedGitRequest::WorktreeAdd {
            git_executable: git_executable.clone(),
            git_common_dir: git_common_dir.clone(),
            base_commit: base_commit.clone(),
            disabled_filter_keys: disabled_filter_keys.clone(),
            expected_target_path: expected_target_path.clone(),
        },
        PinnedGitRequest::Checkout {
            git_executable,
            base_commit,
            disabled_filter_keys,
            expected_target_path,
        } => PinnedGitRequest::Checkout {
            git_executable: git_executable.clone(),
            base_commit: base_commit.clone(),
            disabled_filter_keys: disabled_filter_keys.clone(),
            expected_target_path: expected_target_path.clone(),
        },
        PinnedGitRequest::ReadOnly {
            git_executable,
            command,
            disabled_filter_keys,
            expected_target_path,
        } => PinnedGitRequest::ReadOnly {
            git_executable: git_executable.clone(),
            command: command.clone(),
            disabled_filter_keys: disabled_filter_keys.clone(),
            expected_target_path: expected_target_path.clone(),
        },
    }
}

#[cfg(unix)]
pub(super) fn verify_pinned_target_chain(
    request: &PinnedGitRequest,
    directories: &[fs::File],
) -> Result<(), String> {
    let expected_target = request.expected_target_path();
    if !expected_target.is_absolute() {
        return Err("pinned Git target path must be absolute".to_string());
    }
    if matches!(request, PinnedGitRequest::ReadOnly { .. }) {
        if directories.len() != 1 {
            return Err(
                "pinned read-only Git operation did not carry its exact target handle".to_string(),
            );
        }
        return verify_named_pinned_directory(
            directories
                .first()
                .ok_or_else(|| "pinned read-only Git target handle was missing".to_string())?,
            expected_target,
        );
    }
    if directories.len() != 4 {
        return Err("pinned Git operation did not carry its complete nest chain".to_string());
    }
    let worktree_id = expected_target
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "pinned Git target had no valid worktree id".to_string())?;
    validate_worktree_id(worktree_id)?;
    let repository_identity = expected_target
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .ok_or_else(|| "pinned Git target had no repository identity".to_string())?;
    validate_repository_identity(repository_identity)?;
    if expected_target
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        != Some(WORKTREES_DIRECTORY)
    {
        return Err("pinned Git target was outside the WORKTREES boundary".to_string());
    }

    for (index, directory) in directories.iter().enumerate() {
        let path = match index {
            0 => expected_target
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent),
            1 => expected_target.parent().and_then(Path::parent),
            2 => expected_target.parent(),
            3 => Some(expected_target),
            _ => None,
        }
        .ok_or_else(|| "pinned Git target chain was incomplete".to_string())?;
        verify_named_pinned_directory(directory, path)?;
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn verify_named_pinned_directory(
    directory: &fs::File,
    path: &Path,
) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;

    let metadata = directory
        .metadata()
        .map_err(|error| format!("failed to inspect pinned Git directory: {error}"))?;
    if !metadata.is_dir() {
        return Err("pinned Git operation contained a non-directory handle".to_string());
    }
    let named = path.symlink_metadata().map_err(|error| {
        format!(
            "failed to verify named pinned Git directory {}: {error}",
            path.display()
        )
    })?;
    if named.file_type().is_symlink()
        || !named.is_dir()
        || named.dev() != metadata.dev()
        || named.ino() != metadata.ino()
    {
        return Err(format!(
            "pinned Git directory {} moved or was replaced",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(all(unix, test))]
pub(super) fn validate_helper_git_executable(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if !path.is_absolute() {
        return Err("pinned Git executable must be absolute".to_string());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize pinned Git executable: {error}"))?;
    if canonical != path || !canonical.is_file() {
        return Err("pinned Git executable is not a canonical regular file".to_string());
    }
    Ok(canonical)
}

#[cfg(unix)]
pub(super) fn validate_filter_override_keys(keys: &[String]) -> Result<(), String> {
    if keys.len() > MAX_PINNED_GIT_FILTER_KEYS {
        return Err(format!(
            "pinned Git filter overrides exceeded the {MAX_PINNED_GIT_FILTER_KEYS}-key limit"
        ));
    }
    for key in keys {
        let normalized = key.to_ascii_lowercase();
        let valid_suffix = [".clean", ".smudge", ".process", ".required"]
            .iter()
            .any(|suffix| normalized.ends_with(suffix));
        if key.len() > 4096
            || key.chars().any(char::is_control)
            || !normalized.starts_with("filter.")
            || !valid_suffix
        {
            return Err("pinned Git filter override was invalid".to_string());
        }
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn run_git_executable_with_filter_overrides(
    executable: &Path,
    cwd: &Path,
    args: &[OsString],
    operation: GitOperation,
    disabled_filter_keys: &[String],
    deadline: Option<Instant>,
) -> Result<Vec<u8>, String> {
    let mut command = Command::new(executable);
    command.arg("--no-pager").args(args).current_dir(cwd);
    configure_git_environment(&mut command, operation, disabled_filter_keys);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::util::configure_no_window(&mut command);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let timeout = remaining_git_timeout(deadline)?;
    let child = command
        .spawn()
        .map_err(|error| format!("failed to start git: {error}"))?;
    capture_child(child, "git", timeout)
}
