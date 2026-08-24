use super::*;

#[test]
fn prepares_detached_head_without_mutating_the_original_checkout() -> Result<(), String> {
    let repository = create_repository()?;
    let original_head = test_line(&test_git(&repository.root, &["rev-parse", "HEAD"])?);
    let original_branch = test_line(&test_git(&repository.root, &["branch", "--show-current"])?);
    let original_status = test_git(&repository.root, &["status", "--porcelain=v1", "-z"])?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;

    let prepared = prepare_execution_root(
        CodeWorktreePrepareInput {
            repository_root: path_to_string(&repository.root, "test repository")?,
            base_ref: "HEAD".to_string(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        nest.path(),
    )?;

    assert_eq!(prepared.descriptor.base_ref, original_head);
    assert_eq!(prepared.head_commit, original_head);
    assert_eq!(prepared.branch, None);
    assert!(!prepared.dirty);
    let worktree_id = prepared
        .descriptor
        .worktree_id
        .as_deref()
        .ok_or_else(|| "expected a managed worktree id".to_string())?;
    let expected_root = nest
        .path()
        .canonicalize()
        .map_err(|error| error.to_string())?
        .join(WORKTREES_DIRECTORY)
        .join(&prepared.descriptor.repository_identity)
        .join(worktree_id);
    assert_eq!(
        Path::new(&prepared.descriptor.execution_root),
        expected_root
    );
    assert!(test_git(&expected_root, &["symbolic-ref", "-q", "HEAD"]).is_err());
    assert_eq!(
        test_line(&test_git(&repository.root, &["rev-parse", "HEAD"])?),
        original_head
    );
    assert_eq!(
        test_line(&test_git(&repository.root, &["branch", "--show-current"])?),
        original_branch
    );
    assert_eq!(
        test_git(&repository.root, &["status", "--porcelain=v1", "-z"])?,
        original_status
    );
    assert_eq!(
        revalidate_execution_root(&prepared.descriptor, nest.path())?.head_commit,
        original_head
    );
    Ok(())
}

#[test]
fn captures_only_same_commit_direct_local_branch_authority() -> Result<(), String> {
    let repository = create_repository()?;
    let repository_info = discover_repository(&repository.root)?;
    let head = resolve_commit(&repository.root, "HEAD")?;
    assert_eq!(
        capture_direct_local_merge_target(&repository_info, "HEAD", &head)?.as_deref(),
        Some("refs/heads/main")
    );
    assert_eq!(
        capture_direct_local_merge_target(&repository_info, "main", &head)?.as_deref(),
        Some("refs/heads/main")
    );
    assert_eq!(
        capture_direct_local_merge_target(&repository_info, "refs/heads/main", &head)?.as_deref(),
        Some("refs/heads/main")
    );

    test_git(&repository.root, &["tag", "schoolx-tag"])?;
    test_git(
        &repository.root,
        &["update-ref", "refs/remotes/origin/main", &head],
    )?;
    for rejected in [
        "schoolx-tag",
        "refs/tags/schoolx-tag",
        "refs/remotes/origin/main",
        "origin/HEAD",
        "main~0",
        head.as_str(),
    ] {
        assert_eq!(
            capture_direct_local_merge_target(&repository_info, rejected, &head)?,
            None,
            "unexpected authority for {rejected}"
        );
    }

    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let prepared = prepare_execution_root_with_merge_target(
        CodeWorktreePrepareInput {
            repository_root: path_to_string(&repository.root, "test repository")?,
            base_ref: "HEAD".to_string(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        nest.path(),
    )?;
    assert_eq!(
        prepared.merge_target_ref.as_deref(),
        Some("refs/heads/main")
    );

    test_git(&repository.root, &["checkout", "--detach", "HEAD"])?;
    let detached = discover_repository(&repository.root)?;
    assert_eq!(
        capture_direct_local_merge_target(&detached, "HEAD", &head)?,
        None
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn repository_filters_cannot_execute_during_prepare_or_status() -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;

    let repository = create_repository()?;
    fs::write(repository.root.join("filtered.txt"), "first\n")
        .map_err(|error| error.to_string())?;
    fs::write(repository.root.join("worktree-only.txt"), "worktree\n")
        .map_err(|error| error.to_string())?;
    fs::write(repository.root.join("conditional.txt"), "conditional\n")
        .map_err(|error| error.to_string())?;
    fs::write(
        repository.root.join(".gitattributes"),
        concat!(
            "filtered.txt filter=schoolxevil\n",
            "worktree-only.txt filter=schoolxworktree\n",
            "conditional.txt filter=schoolxconditional\n",
        ),
    )
    .map_err(|error| error.to_string())?;
    test_git(
        &repository.root,
        &[
            "add",
            ".gitattributes",
            "filtered.txt",
            "worktree-only.txt",
            "conditional.txt",
        ],
    )?;
    test_git(
        &repository.root,
        &[
            "-c",
            "user.name=SchoolX Test",
            "-c",
            "user.email=schoolx@example.invalid",
            "commit",
            "-m",
            "filtered fixture",
        ],
    )?;

    let marker = repository.root.join("filter-executed");
    let script = repository.root.join("filter-driver.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\ntouch '{}'\ncat\n", marker.display()),
    )
    .map_err(|error| error.to_string())?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    test_git(
        &repository.root,
        &[
            "config",
            "filter.schoolxevil.smudge",
            &script.to_string_lossy(),
        ],
    )?;
    test_git(
        &repository.root,
        &[
            "config",
            "filter.schoolxevil.clean",
            &script.to_string_lossy(),
        ],
    )?;
    test_git(
        &repository.root,
        &["config", "filter.schoolxevil.required", "true"],
    )?;
    test_git(
        &repository.root,
        &["config", "extensions.worktreeConfig", "true"],
    )?;
    test_git(
        &repository.root,
        &[
            "config",
            "--worktree",
            "filter.schoolxworktree.process",
            &script.to_string_lossy(),
        ],
    )?;
    let common_dir = test_line(&test_git(
        &repository.root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?);
    let conditional_config = repository.root.join("conditional-filter.conf");
    test_git(
        repository.root.as_path(),
        &[
            "config",
            "-f",
            &conditional_config.to_string_lossy(),
            "filter.schoolxconditional.process",
            &script.to_string_lossy(),
        ],
    )?;
    test_git(
        &repository.root,
        &[
            "config",
            "-f",
            &conditional_config.to_string_lossy(),
            "filter.schoolxconditional.required",
            "true",
        ],
    )?;
    let conditional_key = format!("includeIf.gitdir:{common_dir}/worktrees/**.path");
    test_git(
        &repository.root,
        &[
            "config",
            &conditional_key,
            &conditional_config.to_string_lossy(),
        ],
    )?;

    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let prepared = prepare_execution_root(
        CodeWorktreePrepareInput {
            repository_root: path_to_string(&repository.root, "test repository")?,
            base_ref: "HEAD".to_string(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        nest.path(),
    )?;
    assert!(!marker.exists());

    fs::write(
        Path::new(&prepared.descriptor.execution_root).join("filtered.txt"),
        "other\n",
    )
    .map_err(|error| error.to_string())?;
    let status = revalidate_execution_root(&prepared.descriptor, nest.path())?;
    assert!(status.dirty);
    assert!(!marker.exists());
    Ok(())
}
