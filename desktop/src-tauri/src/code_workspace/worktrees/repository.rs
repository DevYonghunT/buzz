#[cfg(unix)]
use super::pinned_command::*;
use super::{git::*, *};

pub(super) fn discover_repository(candidate: &Path) -> Result<RepositoryInfo, String> {
    discover_repository_until(candidate, None)
}

pub(super) fn discover_repository_until(
    candidate: &Path,
    deadline: Option<Instant>,
) -> Result<RepositoryInfo, String> {
    let candidate = canonical_existing_directory(candidate, "Git repository path")?;
    #[cfg(unix)]
    let top_level_output = run_pinned_read_until(
        &candidate,
        CodePinnedReadCommand::TopLevel,
        Vec::new(),
        deadline,
    )?;
    #[cfg(not(unix))]
    let top_level_output = run_git_until(
        &candidate,
        &[
            OsString::from("rev-parse"),
            OsString::from("--path-format=absolute"),
            OsString::from("--show-toplevel"),
        ],
        GitOperation::ReadOnly,
        deadline,
    )?;
    let top_level = single_path_output(&top_level_output, "Git top-level")?;
    let top_level = canonical_existing_directory(&top_level, "Git top-level")?;

    #[cfg(unix)]
    let common_dir_output = run_pinned_read_until(
        &top_level,
        CodePinnedReadCommand::CommonDir,
        Vec::new(),
        deadline,
    )?;
    #[cfg(not(unix))]
    let common_dir_output = run_git_until(
        &top_level,
        &[
            OsString::from("rev-parse"),
            OsString::from("--path-format=absolute"),
            OsString::from("--git-common-dir"),
        ],
        GitOperation::ReadOnly,
        deadline,
    )?;
    let common_dir = single_path_output(&common_dir_output, "Git common directory")?;
    let common_dir = canonical_existing_directory(&common_dir, "Git common directory")?;
    let identity = repository_identity(&common_dir)?;

    Ok(RepositoryInfo {
        top_level,
        common_dir,
        identity,
    })
}

pub(super) fn resolve_commit(repository_root: &Path, base_ref: &str) -> Result<String, String> {
    resolve_commit_until(repository_root, base_ref, None)
}

pub(super) fn resolve_commit_until(
    repository_root: &Path,
    base_ref: &str,
    deadline: Option<Instant>,
) -> Result<String, String> {
    let base_ref = validate_base_ref(base_ref)?;
    #[cfg(unix)]
    let output = run_pinned_read_until(
        repository_root,
        CodePinnedReadCommand::ResolveCommit {
            base_ref: base_ref.to_string(),
        },
        Vec::new(),
        deadline,
    )
    .map_err(|error| format!("failed to resolve SchoolX Code base ref `{base_ref}`: {error}"))?;
    #[cfg(not(unix))]
    let output = {
        let revision = format!("{base_ref}^{{commit}}");
        run_git_until(
            repository_root,
            &[
                OsString::from("rev-parse"),
                OsString::from("--verify"),
                OsString::from("--quiet"),
                OsString::from("--end-of-options"),
                OsString::from(revision),
            ],
            GitOperation::ReadOnly,
            deadline,
        )
        .map_err(|error| format!("failed to resolve SchoolX Code base ref `{base_ref}`: {error}"))?
    };
    let commit = single_text_output(&output, "resolved Git commit")?;
    validate_commit_id(&commit)?;
    Ok(commit)
}

pub(super) fn repository_is_dirty_until(
    repository_root: &Path,
    deadline: Option<Instant>,
) -> Result<bool, String> {
    #[cfg(unix)]
    let output = run_pinned_read_until(
        repository_root,
        CodePinnedReadCommand::StatusPorcelain,
        repository_filter_overrides_until(repository_root, deadline)?,
        deadline,
    )?;
    #[cfg(not(unix))]
    let output = run_git_until(
        repository_root,
        &[
            OsString::from("status"),
            OsString::from("--porcelain=v1"),
            OsString::from("-z"),
            OsString::from("--untracked-files=normal"),
        ],
        GitOperation::WorkingTreeRead,
        deadline,
    )?;
    Ok(!output.is_empty())
}

pub(super) fn repository_branch_until(
    repository_root: &Path,
    deadline: Option<Instant>,
) -> Result<Option<String>, String> {
    #[cfg(unix)]
    let output = run_pinned_read_until(
        repository_root,
        CodePinnedReadCommand::CurrentBranch,
        Vec::new(),
        deadline,
    )?;
    #[cfg(not(unix))]
    let output = run_git_until(
        repository_root,
        &[OsString::from("branch"), OsString::from("--show-current")],
        GitOperation::ReadOnly,
        deadline,
    )?;
    let branch = std::str::from_utf8(&output)
        .map_err(|error| format!("current Git branch was not UTF-8: {error}"))?
        .trim_end_matches(['\r', '\n']);
    if branch.is_empty() {
        return Ok(None);
    }
    if branch.len() > MAX_BASE_REF_BYTES
        || branch
            .chars()
            .any(|character| character.is_control() || character == '\0')
    {
        return Err("current Git branch contained an unsafe value".to_string());
    }
    Ok(Some(branch.to_string()))
}

pub(crate) fn repository_identity(common_dir: &Path) -> Result<String, String> {
    let common_dir = path_to_string(common_dir, "Git common directory")?;
    let mut hasher = Sha256::new();
    hasher.update(b"schoolx-code-repository-v1\0");
    hasher.update(common_dir.as_bytes());
    Ok(hex::encode(hasher.finalize()))
}

pub(super) fn validate_base_ref(value: &str) -> Result<&str, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_BASE_REF_BYTES {
        return Err(format!(
            "SchoolX Code base ref must be between 1 and {MAX_BASE_REF_BYTES} bytes"
        ));
    }
    if value.starts_with('-')
        || value
            .chars()
            .any(|character| character.is_control() || character == '\0')
    {
        return Err("SchoolX Code base ref contains unsafe characters".to_string());
    }
    Ok(value)
}

pub(super) fn validate_repository_identity(value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("SchoolX Code repository identity must be 64 lowercase hex characters".into());
    }
    Ok(())
}

pub(super) fn validate_commit_id(value: &str) -> Result<(), String> {
    if !matches!(value.len(), 40 | 64)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("SchoolX Code base commit must be a full lowercase Git object id".to_string());
    }
    Ok(())
}

pub(super) fn validate_worktree_id(value: &str) -> Result<(), String> {
    let parsed = Uuid::parse_str(value)
        .map_err(|error| format!("invalid SchoolX Code worktree id: {error}"))?;
    if parsed.to_string() != value {
        return Err("SchoolX Code worktree id is not canonical".to_string());
    }
    Ok(())
}

pub(super) fn canonical_existing_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{label} must be an absolute path"));
    }
    let metadata = path
        .symlink_metadata()
        .map_err(|error| format!("failed to inspect {label} {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("{label} {} must not be a symlink", path.display()));
    }
    if !metadata.is_dir() {
        return Err(format!("{label} {} must be a directory", path.display()));
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize {label} {}: {error}", path.display()))?;
    let canonical_metadata = canonical.symlink_metadata().map_err(|error| {
        format!(
            "failed to revalidate canonical {label} {}: {error}",
            canonical.display()
        )
    })?;
    if canonical_metadata.file_type().is_symlink() || !canonical_metadata.is_dir() {
        return Err(format!(
            "canonical {label} {} is not a real directory",
            canonical.display()
        ));
    }
    Ok(canonical)
}

pub(super) fn ensure_real_child_directory(parent: &Path, name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Err("invalid SchoolX Code managed-directory component".to_string());
    }
    let child = parent.join(name);
    match child.symlink_metadata() {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "SchoolX Code managed directory {} must not be a symlink",
                    child.display()
                ));
            }
            if !metadata.is_dir() {
                return Err(format!(
                    "SchoolX Code managed path {} must be a directory",
                    child.display()
                ));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match create_private_directory(&child) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(format!(
                        "failed to create SchoolX Code managed directory {}: {error}",
                        child.display()
                    ));
                }
            }
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect SchoolX Code managed directory {}: {error}",
                child.display()
            ));
        }
    }
    let canonical = canonical_existing_directory(&child, "SchoolX Code managed directory")?;
    if canonical.parent() != Some(parent) {
        return Err(format!(
            "SchoolX Code managed directory {} escaped its parent",
            child.display()
        ));
    }
    // Existing buckets may have been created by an older build or a permissive
    // umask. Re-apply the product boundary on every use, not just creation.
    set_owner_only_directory(&canonical)?;
    Ok(canonical)
}

pub(super) fn reserve_worktree_target(parent: &Path) -> Result<(String, PathBuf), String> {
    for _ in 0..WORKTREE_ID_ATTEMPTS {
        let worktree_id = Uuid::new_v4().to_string();
        let target = parent.join(&worktree_id);
        match create_private_directory(&target) {
            Ok(()) => {
                validate_reserved_worktree_target(parent, &target)?;
                return Ok((worktree_id, target));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "failed to reserve SchoolX Code worktree target {}: {error}",
                    target.display()
                ));
            }
        }
    }
    Err("failed to allocate an unused SchoolX Code worktree id".to_string())
}

pub(super) fn validate_reserved_worktree_target(
    parent: &Path,
    target: &Path,
) -> Result<(), String> {
    let metadata = target.symlink_metadata().map_err(|error| {
        format!(
            "failed to inspect reserved SchoolX Code worktree target {}: {error}",
            target.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "reserved SchoolX Code worktree target {} is not a real directory",
            target.display()
        ));
    }
    let canonical = target.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize reserved SchoolX Code worktree target {}: {error}",
            target.display()
        )
    })?;
    if canonical != target || canonical.parent() != Some(parent) {
        return Err(format!(
            "reserved SchoolX Code worktree target {} escaped its parent",
            target.display()
        ));
    }
    let mut entries = fs::read_dir(&canonical).map_err(|error| {
        format!(
            "failed to read reserved SchoolX Code worktree target {}: {error}",
            canonical.display()
        )
    })?;
    match entries.next() {
        None => Ok(()),
        Some(Ok(_)) => Err(format!(
            "reserved SchoolX Code worktree target {} is not empty",
            canonical.display()
        )),
        Some(Err(error)) => Err(format!(
            "failed to inspect reserved SchoolX Code worktree target {}: {error}",
            canonical.display()
        )),
    }
}

#[cfg(unix)]
pub(super) fn create_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700).create(path)
}

#[cfg(not(unix))]
pub(super) fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir(path)
}

pub(super) fn validate_managed_execution_root(
    nest_root: &Path,
    repository_identity: &str,
    worktree_id: &str,
    claimed_root: &Path,
) -> Result<PathBuf, String> {
    validate_repository_identity(repository_identity)?;
    validate_worktree_id(worktree_id)?;
    let nest_root = canonical_existing_directory(nest_root, "SchoolX nest root")?;
    let worktrees_root = existing_real_child_directory(&nest_root, WORKTREES_DIRECTORY)?;
    let repository_root = existing_real_child_directory(&worktrees_root, repository_identity)?;
    let expected_root = repository_root.join(worktree_id);
    if claimed_root != expected_root {
        return Err("SchoolX Code worktree path does not match its managed descriptor".to_string());
    }
    let execution_root = existing_real_child_directory(&repository_root, worktree_id)?;
    if execution_root != expected_root || !execution_root.starts_with(&worktrees_root) {
        return Err("SchoolX Code worktree escaped the active nest boundary".to_string());
    }
    Ok(execution_root)
}

pub(super) fn existing_real_child_directory(parent: &Path, name: &str) -> Result<PathBuf, String> {
    let child = parent.join(name);
    let canonical = canonical_existing_directory(&child, "SchoolX Code managed directory")?;
    if canonical.parent() != Some(parent) {
        return Err(format!(
            "SchoolX Code managed directory {} escaped its parent",
            child.display()
        ));
    }
    Ok(canonical)
}

#[cfg(unix)]
pub(super) fn set_owner_only_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::fs::PermissionsExt;

    let directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| format!("failed to securely open {}: {error}", path.display()))?;
    let metadata = directory.metadata().map_err(|error| {
        format!(
            "failed to inspect open directory {}: {error}",
            path.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }
    directory
        .set_permissions(fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("failed to restrict {}: {error}", path.display()))
}

#[cfg(not(unix))]
pub(super) fn set_owner_only_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

pub(super) fn path_to_string(path: &Path, label: &str) -> Result<String, String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{label} is not valid UTF-8"))
}

pub(super) fn single_path_output(output: &[u8], label: &str) -> Result<PathBuf, String> {
    single_text_output(output, label).map(PathBuf::from)
}

pub(super) fn single_text_output(output: &[u8], label: &str) -> Result<String, String> {
    let output =
        std::str::from_utf8(output).map_err(|error| format!("{label} was not UTF-8: {error}"))?;
    let output = output.trim_end_matches(['\r', '\n']);
    if output.is_empty() || output.contains('\n') || output.contains('\r') {
        return Err(format!("{label} did not contain exactly one value"));
    }
    Ok(output.to_string())
}
