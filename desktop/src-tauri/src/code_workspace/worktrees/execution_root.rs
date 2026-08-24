use super::{git::*, repository::*, *};
#[cfg(unix)]
use super::{pinned_command::*, pinned_operation::*};

#[cfg(test)]
pub(super) fn inspect_repository(
    repository_root: &str,
) -> Result<CodeRepositoryDescriptor, String> {
    let repository = discover_repository(Path::new(repository_root))?;
    repository_descriptor(&repository)
}

/// Resolve `base_ref` and prepare an execution root below the supplied active
/// SchoolX nest. Local mode deliberately does not inspect or create the nest.
pub fn prepare_execution_root(
    input: CodeWorktreePrepareInput,
    nest_root: &Path,
) -> Result<CodeWorktreePrepareResult, String> {
    prepare_execution_root_with_merge_target(input, nest_root).map(|prepared| prepared.worktree)
}

/// Prepare an execution root while capturing optional direct-local merge
/// authority before the first managed-worktree mutation.
pub(crate) fn prepare_execution_root_with_merge_target(
    input: CodeWorktreePrepareInput,
    nest_root: &Path,
) -> Result<CodePreparedExecutionRoot, String> {
    #[cfg(unix)]
    {
        with_pinned_git_authority(|| {
            prepare_execution_root_with_merge_target_inner(input, nest_root)
        })
    }
    #[cfg(not(unix))]
    prepare_execution_root_with_merge_target_inner(input, nest_root)
}

pub(super) fn prepare_execution_root_with_merge_target_inner(
    input: CodeWorktreePrepareInput,
    nest_root: &Path,
) -> Result<CodePreparedExecutionRoot, String> {
    #[cfg(not(unix))]
    if input.execution_mode == CodeExecutionMode::Worktree {
        return Err(
            "SchoolX Code managed worktree launch is unsupported on this platform".to_string(),
        );
    }

    let repository = discover_repository(Path::new(&input.repository_root))?;
    let repository_descriptor = repository_descriptor(&repository)?;
    let base_commit = resolve_commit(&repository.top_level, &input.base_ref)?;
    let merge_target_ref = if input.execution_mode == CodeExecutionMode::Worktree {
        capture_direct_local_merge_target(&repository, &input.base_ref, &base_commit)?
    } else {
        None
    };
    let status = match input.execution_mode {
        CodeExecutionMode::Local => {
            let descriptor = CodeWorktreeDescriptor {
                execution_mode: CodeExecutionMode::Local,
                repository_identity: repository.identity.clone(),
                execution_root: path_to_string(&repository.top_level, "execution root")?,
                base_ref: base_commit,
                worktree_id: None,
            };
            revalidate_execution_root(&descriptor, nest_root)?
        }
        CodeExecutionMode::Worktree => {
            #[cfg(unix)]
            {
                let probe_directory = pin_git_directory(&repository.top_level)?;
                let launch = PinnedGitLaunchAuthority::admit(&probe_directory)?;
                let descriptor =
                    prepare_managed_worktree(&repository, &base_commit, nest_root, &launch)?;
                revalidate_execution_root(&descriptor, nest_root)?
            }
            #[cfg(not(unix))]
            {
                return Err(
                    "SchoolX Code managed worktree launch is unsupported on this platform"
                        .to_string(),
                );
            }
        }
    };

    Ok(CodePreparedExecutionRoot {
        worktree: CodeWorktreePrepareResult {
            repository: repository_descriptor,
            descriptor: status.descriptor,
            head_commit: status.head_commit,
            branch: status.branch,
            dirty: status.dirty,
        },
        merge_target_ref,
    })
}

pub(super) fn capture_direct_local_merge_target(
    repository: &RepositoryInfo,
    selected_base: &str,
    resolved_base: &str,
) -> Result<Option<String>, String> {
    validate_commit_id(resolved_base)?;
    let selected_base = validate_base_ref(selected_base)?;
    let target_ref = if selected_base == "HEAD" {
        let Some(branch) = repository_branch_until(&repository.top_level, None)? else {
            return Ok(None);
        };
        format!("refs/heads/{branch}")
    } else if selected_base.starts_with("refs/heads/") {
        selected_base.to_string()
    } else {
        if selected_base.starts_with("refs/") || validate_commit_id(selected_base).is_ok() {
            return Ok(None);
        }
        format!("refs/heads/{selected_base}")
    };
    if validate_direct_local_branch_ref(&target_ref).is_err() {
        return Ok(None);
    }
    let Ok(target_commit) = resolve_commit(&repository.top_level, &target_ref) else {
        return Ok(None);
    };
    if target_commit == resolved_base {
        Ok(Some(target_ref))
    } else {
        Ok(None)
    }
}

/// Inspect a selected checkout and resolve its base ref without creating a
/// managed worktree. The command facade uses this preflight to validate the
/// caller's repository scope before any Git mutation.
pub fn preflight_execution_root(
    repository_root: &str,
    base_ref: &str,
) -> Result<CodeRepositoryDescriptor, String> {
    #[cfg(unix)]
    {
        with_pinned_git_authority(|| preflight_execution_root_inner(repository_root, base_ref))
    }
    #[cfg(not(unix))]
    preflight_execution_root_inner(repository_root, base_ref)
}

pub(super) fn preflight_execution_root_inner(
    repository_root: &str,
    base_ref: &str,
) -> Result<CodeRepositoryDescriptor, String> {
    let repository = discover_repository(Path::new(repository_root))?;
    resolve_commit(&repository.top_level, base_ref)?;
    repository_descriptor(&repository)
}

/// Revalidate a persisted descriptor against the current filesystem, nest
/// boundary, Git common directory, and repository identity before reuse.
pub fn revalidate_execution_root(
    descriptor: &CodeWorktreeDescriptor,
    nest_root: &Path,
) -> Result<CodeWorktreeStatus, String> {
    revalidate_execution_root_with_authority(descriptor, nest_root, None)
}

/// Revalidate a persisted descriptor while bounding all Git subprocesses by
/// one caller-owned deadline.
pub(crate) fn revalidate_execution_root_before(
    descriptor: &CodeWorktreeDescriptor,
    nest_root: &Path,
    deadline: Instant,
) -> Result<CodeWorktreeStatus, String> {
    revalidate_execution_root_with_authority(descriptor, nest_root, Some(deadline))
}

pub(super) fn revalidate_execution_root_with_authority(
    descriptor: &CodeWorktreeDescriptor,
    nest_root: &Path,
    deadline: Option<Instant>,
) -> Result<CodeWorktreeStatus, String> {
    #[cfg(unix)]
    {
        with_pinned_git_authority(|| {
            revalidate_execution_root_until(descriptor, nest_root, deadline)
        })
    }
    #[cfg(not(unix))]
    revalidate_execution_root_until(descriptor, nest_root, deadline)
}

pub(super) fn revalidate_execution_root_until(
    descriptor: &CodeWorktreeDescriptor,
    nest_root: &Path,
    deadline: Option<Instant>,
) -> Result<CodeWorktreeStatus, String> {
    validate_repository_identity(&descriptor.repository_identity)?;
    validate_commit_id(&descriptor.base_ref)?;

    let execution_root = match descriptor.execution_mode {
        CodeExecutionMode::Local => {
            if descriptor.worktree_id.is_some() {
                return Err("local SchoolX Code execution cannot have a worktree id".to_string());
            }
            let execution_root = canonical_existing_directory(
                Path::new(&descriptor.execution_root),
                "SchoolX Code local execution root",
            )?;
            execution_root
        }
        CodeExecutionMode::Worktree => {
            let worktree_id = descriptor.worktree_id.as_deref().ok_or_else(|| {
                "managed SchoolX Code execution requires a worktree id".to_string()
            })?;
            validate_worktree_id(worktree_id)?;
            validate_managed_execution_root(
                nest_root,
                &descriptor.repository_identity,
                worktree_id,
                Path::new(&descriptor.execution_root),
            )?
        }
    };

    let execution_repository = discover_repository_until(&execution_root, deadline)?;
    if execution_repository.top_level != execution_root {
        return Err("SchoolX Code execution root is not a Git top-level directory".to_string());
    }
    if execution_repository.identity != descriptor.repository_identity {
        return Err("SchoolX Code execution root belongs to a different repository".to_string());
    }
    let resolved_base = resolve_commit_until(&execution_root, &descriptor.base_ref, deadline)?;
    if resolved_base != descriptor.base_ref {
        return Err("stored SchoolX Code base commit changed".to_string());
    }

    let canonical_execution = path_to_string(&execution_root, "execution root")?;
    if canonical_execution != descriptor.execution_root {
        return Err("stored SchoolX Code execution root is not canonical".to_string());
    }
    let head_commit = resolve_commit_until(&execution_root, "HEAD", deadline)?;
    let branch = repository_branch_until(&execution_root, deadline)?;
    let dirty = repository_is_dirty_until(&execution_root, deadline)?;

    Ok(CodeWorktreeStatus {
        descriptor: descriptor.clone(),
        head_commit,
        branch,
        dirty,
    })
}

/// Prove ancestry only from one exact binding-store snapshot. Missing native
/// authority remains a closed `None`; no ref is inferred from `base_ref`.
#[allow(dead_code)]
pub(crate) fn prove_binding_merge_target_before(
    store: &CodeThreadBindingStore,
    input: &CodeThreadBindingLookupInput,
    nest_root: &Path,
    deadline: Instant,
) -> Result<Option<CodeMergeProofOutcome>, String> {
    let Some((binding, target_ref)) = store.binding_merge_authority(input)? else {
        return Ok(None);
    };
    let Some(target_ref) = target_ref else {
        return Ok(None);
    };
    let descriptor = CodeWorktreeDescriptor {
        execution_mode: binding.execution_mode,
        repository_identity: binding.repository_identity,
        execution_root: binding.execution_root,
        base_ref: binding.base_ref,
        worktree_id: binding.worktree_id,
    };
    #[cfg(all(target_os = "macos", not(test)))]
    {
        with_pinned_git_authority(|| {
            prove_direct_local_ancestry_before(&descriptor, nest_root, &target_ref, deadline)
                .map(Some)
        })
    }
    #[cfg(any(not(target_os = "macos"), test))]
    prove_direct_local_ancestry_before(&descriptor, nest_root, &target_ref, deadline).map(Some)
}

/// Run one bounded, read-only graph proof against a persisted direct local ref.
/// Exit 0 from `merge-base --is-ancestor` is the only positive result; exit 1
/// is a stable negative and every other condition is unavailable/error.
#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn prove_direct_local_ancestry_before(
    descriptor: &CodeWorktreeDescriptor,
    nest_root: &Path,
    target_ref: &str,
    deadline: Instant,
) -> Result<CodeMergeProofOutcome, String> {
    prove_direct_local_ancestry_with_hook(descriptor, nest_root, target_ref, deadline, || Ok(()))
}

#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn prove_direct_local_ancestry_with_hook<F>(
    descriptor: &CodeWorktreeDescriptor,
    nest_root: &Path,
    target_ref: &str,
    deadline: Instant,
    after_ancestry: F,
) -> Result<CodeMergeProofOutcome, String>
where
    F: FnOnce() -> Result<(), String>,
{
    with_pinned_git_authority(|| {
        prove_direct_local_ancestry_with_hook_inner(
            descriptor,
            nest_root,
            target_ref,
            deadline,
            after_ancestry,
        )
    })
}

#[cfg(unix)]
pub(super) fn prove_direct_local_ancestry_with_hook_inner<F>(
    descriptor: &CodeWorktreeDescriptor,
    nest_root: &Path,
    target_ref: &str,
    deadline: Instant,
    after_ancestry: F,
) -> Result<CodeMergeProofOutcome, String>
where
    F: FnOnce() -> Result<(), String>,
{
    validate_direct_local_branch_ref(target_ref)?;
    if descriptor.execution_mode != CodeExecutionMode::Worktree {
        return Err("SchoolX Code merge proof requires a managed worktree".to_string());
    }
    let worktree_id = descriptor
        .worktree_id
        .as_deref()
        .ok_or_else(|| "SchoolX Code merge proof is missing its worktree id".to_string())?;
    validate_worktree_id(worktree_id)?;

    let first = read_merge_proof_snapshot(descriptor, nest_root, target_ref, deadline)?;
    let is_ancestor = run_pinned_ancestry_before(
        Path::new(&descriptor.execution_root),
        &first.head_commit,
        &first.target_commit,
        deadline,
    )?;
    after_ancestry()?;
    let second = read_merge_proof_snapshot(descriptor, nest_root, target_ref, deadline)?;
    if first != second {
        return Err("SchoolX Code merge proof inputs changed during inspection".to_string());
    }
    if !is_ancestor {
        return Ok(CodeMergeProofOutcome::NotMerged);
    }
    Ok(CodeMergeProofOutcome::Proven(CodeMergeProofReceipt {
        repository_identity: descriptor.repository_identity.clone(),
        worktree_id: worktree_id.to_string(),
        head_commit: first.head_commit,
        target_ref: target_ref.to_string(),
        target_commit: first.target_commit,
    }))
}

#[cfg(not(unix))]
#[allow(dead_code)]
pub(crate) fn prove_direct_local_ancestry_before(
    _descriptor: &CodeWorktreeDescriptor,
    _nest_root: &Path,
    _target_ref: &str,
    _deadline: Instant,
) -> Result<CodeMergeProofOutcome, String> {
    Err("SchoolX Code pinned merge proof is unsupported on this platform".to_string())
}

#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn read_merge_proof_snapshot(
    descriptor: &CodeWorktreeDescriptor,
    nest_root: &Path,
    target_ref: &str,
    deadline: Instant,
) -> Result<CodeMergeProofSnapshot, String> {
    if Instant::now() >= deadline {
        return Err("SchoolX Code merge proof budget was exhausted".to_string());
    }
    let status = revalidate_execution_root_before(descriptor, nest_root, deadline)?;
    let execution_root = Path::new(&descriptor.execution_root);
    let repository = discover_repository_until(execution_root, Some(deadline))?;
    if repository.top_level != execution_root
        || repository.identity != descriptor.repository_identity
    {
        return Err("SchoolX Code merge proof repository identity changed".to_string());
    }
    reject_legacy_grafts(&repository.common_dir)?;
    let root = merge_proof_path_identity(execution_root, "managed worktree root")?;
    let common_dir = merge_proof_path_identity(&repository.common_dir, "Git common directory")?;
    let head_output =
        run_pinned_read_before(execution_root, CodePinnedReadCommand::HeadCommit, deadline)?;
    let head_commit = single_text_output(&head_output, "merge-proof HEAD commit")?;
    validate_commit_id(&head_commit)?;
    if head_commit != status.head_commit {
        return Err("SchoolX Code merge-proof HEAD changed during snapshot".to_string());
    }
    let target_output = run_pinned_read_before(
        execution_root,
        CodePinnedReadCommand::DirectLocalRefCommit {
            target_ref: target_ref.to_string(),
        },
        deadline,
    )?;
    let target_commit = single_text_output(&target_output, "merge-proof target commit")?;
    validate_commit_id(&target_commit)?;
    reject_legacy_grafts(&repository.common_dir)?;
    Ok(CodeMergeProofSnapshot {
        head_commit,
        target_commit,
        root,
        common_dir,
    })
}

#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn merge_proof_path_identity(
    path: &Path,
    label: &str,
) -> Result<CodeMergeProofPathIdentity, String> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = path
        .symlink_metadata()
        .map_err(|error| format!("failed to inspect merge-proof {label}: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "SchoolX Code merge-proof {label} is not a real directory"
        ));
    }
    Ok(CodeMergeProofPathIdentity {
        path: path.to_path_buf(),
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn reject_legacy_grafts(common_dir: &Path) -> Result<(), String> {
    let info = common_dir.join("info");
    let info_metadata = match info.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("failed to inspect Git info directory: {error}")),
    };
    if info_metadata.file_type().is_symlink() || !info_metadata.is_dir() {
        return Err("SchoolX Code merge proof rejected an unsafe Git info directory".to_string());
    }
    let grafts = info.join("grafts");
    let graft_metadata = match grafts.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("failed to inspect legacy Git grafts: {error}")),
    };
    if graft_metadata.file_type().is_symlink() || !graft_metadata.is_file() {
        return Err("SchoolX Code merge proof rejected an unsafe legacy graft file".to_string());
    }
    if graft_metadata.len() != 0 {
        return Err("SchoolX Code merge proof rejected non-empty legacy Git grafts".to_string());
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn prepare_managed_worktree(
    repository: &RepositoryInfo,
    base_commit: &str,
    nest_root: &Path,
    launch: &PinnedGitLaunchAuthority,
) -> Result<CodeWorktreeDescriptor, String> {
    let nest_root = canonical_existing_directory(nest_root, "SchoolX nest root")?;
    let worktrees_root = ensure_real_child_directory(&nest_root, WORKTREES_DIRECTORY)?;
    let repository_root = ensure_real_child_directory(&worktrees_root, &repository.identity)?;

    let (worktree_id, target) = reserve_worktree_target(&repository_root)?;
    validate_reserved_worktree_target(&repository_root, &target)?;
    add_managed_worktree(repository, &target, base_commit, launch)?;

    let execution_root =
        validate_managed_execution_root(&nest_root, &repository.identity, &worktree_id, &target)?;
    set_owner_only_directory(&execution_root)?;
    let created_repository = discover_repository(&execution_root)?;
    if created_repository.top_level != execution_root
        || created_repository.common_dir != repository.common_dir
        || created_repository.identity != repository.identity
    {
        return Err(
            "created SchoolX Code worktree failed repository identity validation".to_string(),
        );
    }
    let reserved_head = resolve_commit(&execution_root, "HEAD")?;
    if reserved_head != base_commit {
        return Err("created SchoolX Code worktree is not at the resolved base commit".to_string());
    }
    // `worktree add --no-checkout` creates only Git administrative metadata.
    // Enumerate effective config again from the new worktree context before
    // materializing tracked files so worktree-scoped and conditional filter
    // drivers are disabled by their exact keys.
    checkout_managed_worktree(&execution_root, base_commit, launch)?;

    Ok(CodeWorktreeDescriptor {
        execution_mode: CodeExecutionMode::Worktree,
        repository_identity: repository.identity.clone(),
        execution_root: path_to_string(&execution_root, "execution root")?,
        base_ref: base_commit.to_string(),
        worktree_id: Some(worktree_id),
    })
}

#[cfg(unix)]
pub(super) fn add_managed_worktree(
    repository: &RepositoryInfo,
    target: &Path,
    base_commit: &str,
    launch: &PinnedGitLaunchAuthority,
) -> Result<(), String> {
    let request = PinnedGitRequest::WorktreeAdd {
        git_executable: path_to_string(&launch.git_executable()?, "git executable")?,
        git_common_dir: path_to_string(&repository.common_dir, "Git common directory")?,
        base_commit: base_commit.to_string(),
        disabled_filter_keys: repository_filter_overrides(&repository.top_level)?,
        expected_target_path: path_to_string(target, "managed worktree target")?,
    };
    run_git_in_pinned_directory(target, request, launch).map(|_| ())
}

#[cfg(unix)]
pub(super) fn checkout_managed_worktree(
    execution_root: &Path,
    base_commit: &str,
    launch: &PinnedGitLaunchAuthority,
) -> Result<(), String> {
    let request = PinnedGitRequest::Checkout {
        git_executable: path_to_string(&launch.git_executable()?, "git executable")?,
        base_commit: base_commit.to_string(),
        disabled_filter_keys: repository_filter_overrides(execution_root)?,
        expected_target_path: path_to_string(execution_root, "managed worktree target")?,
    };
    run_git_in_pinned_directory(execution_root, request, launch).map(|_| ())
}

pub(super) fn repository_descriptor(
    repository: &RepositoryInfo,
) -> Result<CodeRepositoryDescriptor, String> {
    Ok(CodeRepositoryDescriptor {
        repository_root: path_to_string(&repository.top_level, "repository root")?,
        git_common_dir: path_to_string(&repository.common_dir, "Git common directory")?,
        repository_identity: repository.identity.clone(),
    })
}
