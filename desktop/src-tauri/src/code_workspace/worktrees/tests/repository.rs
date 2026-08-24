use super::*;

#[test]
fn resolves_a_named_ref_and_local_mode_does_not_create_the_nest() -> Result<(), String> {
    let repository = create_repository()?;
    let tagged_commit = test_line(&test_git(&repository.root, &["rev-parse", "HEAD"])?);
    test_git(&repository.root, &["tag", "schoolx-base"])?;
    fs::write(repository.root.join("README.md"), "second\n").map_err(|error| error.to_string())?;
    test_git(&repository.root, &["add", "README.md"])?;
    test_git(
        &repository.root,
        &[
            "-c",
            "user.name=SchoolX Test",
            "-c",
            "user.email=schoolx@example.invalid",
            "commit",
            "-m",
            "second",
        ],
    )?;
    let absent_nest_parent = tempfile::tempdir().map_err(|error| error.to_string())?;
    let absent_nest = absent_nest_parent.path().join("must-not-be-created");

    let prepared = prepare_execution_root(
        CodeWorktreePrepareInput {
            repository_root: path_to_string(&repository.root, "test repository")?,
            base_ref: "schoolx-base".to_string(),
            execution_mode: CodeExecutionMode::Local,
        },
        &absent_nest,
    )?;

    assert_eq!(prepared.descriptor.base_ref, tagged_commit);
    assert!(prepared.descriptor.worktree_id.is_none());
    assert_eq!(prepared.branch.as_deref(), Some("main"));
    assert!(!absent_nest.exists());
    assert!(prepare_execution_root(
        CodeWorktreePrepareInput {
            repository_root: path_to_string(&repository.root, "test repository")?,
            base_ref: "refs/heads/does-not-exist".to_string(),
            execution_mode: CodeExecutionMode::Local,
        },
        &absent_nest,
    )
    .is_err());
    assert!(!absent_nest.exists());
    Ok(())
}

#[test]
fn linked_worktrees_share_common_dir_and_repository_identity() -> Result<(), String> {
    let repository = create_repository()?;
    let linked = repository
        ._directory
        .path()
        .join("existing-linked-worktree");
    let linked_string = path_to_string(&linked, "linked test worktree")?;
    test_git(
        &repository.root,
        &["worktree", "add", "--detach", &linked_string, "HEAD"],
    )?;

    let main = inspect_repository(&path_to_string(&repository.root, "test repository")?)?;
    let linked = inspect_repository(&linked_string)?;
    assert_ne!(main.repository_root, linked.repository_root);
    assert_eq!(main.git_common_dir, linked.git_common_dir);
    assert_eq!(main.repository_identity, linked.repository_identity);
    Ok(())
}

#[test]
fn repository_identity_algorithm_is_domain_separated_and_deterministic() -> Result<(), String> {
    assert_eq!(
        repository_identity(Path::new("/canonical/repo/.git"))?,
        "01b765f26c4b7868fe85614a89af89247e4cb02c20f9548200a7c32765050bbe"
    );
    Ok(())
}

#[test]
fn local_status_reports_a_new_untracked_file_as_dirty() -> Result<(), String> {
    let repository = create_repository()?;
    let unused_nest = repository._directory.path().join("unused-nest");
    let prepared = prepare_execution_root(
        CodeWorktreePrepareInput {
            repository_root: path_to_string(&repository.root, "test repository")?,
            base_ref: "HEAD".to_string(),
            execution_mode: CodeExecutionMode::Local,
        },
        &unused_nest,
    )?;
    assert!(!prepared.dirty);
    assert_eq!(prepared.branch.as_deref(), Some("main"));

    fs::write(repository.root.join("untracked.txt"), "untracked\n")
        .map_err(|error| error.to_string())?;
    let status = revalidate_execution_root(&prepared.descriptor, &unused_nest)?;
    assert!(status.dirty);
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert!(!unused_nest.exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn existing_managed_buckets_are_restricted_to_owner_only() -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let repository = create_repository()?;
    let identity = inspect_repository(&path_to_string(&repository.root, "test repository")?)?
        .repository_identity;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let worktrees = nest.path().join(WORKTREES_DIRECTORY);
    let bucket = worktrees.join(&identity);
    fs::create_dir(&worktrees).map_err(|error| error.to_string())?;
    fs::create_dir(&bucket).map_err(|error| error.to_string())?;
    fs::set_permissions(&worktrees, fs::Permissions::from_mode(0o777))
        .map_err(|error| error.to_string())?;
    fs::set_permissions(&bucket, fs::Permissions::from_mode(0o777))
        .map_err(|error| error.to_string())?;

    prepare_execution_root(
        CodeWorktreePrepareInput {
            repository_root: path_to_string(&repository.root, "test repository")?,
            base_ref: "HEAD".to_string(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        nest.path(),
    )?;

    for path in [&worktrees, &bucket] {
        let mode = fs::metadata(path)
            .map_err(|error| error.to_string())?
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }
    Ok(())
}

#[test]
fn status_rejects_a_missing_managed_target() -> Result<(), String> {
    let repository = create_repository()?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let prepared = prepare_execution_root(
        CodeWorktreePrepareInput {
            repository_root: path_to_string(&repository.root, "test repository")?,
            base_ref: "HEAD".to_string(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        nest.path(),
    )?;
    let original = PathBuf::from(&prepared.descriptor.execution_root);
    let moved = original.with_extension("moved-for-test");
    fs::rename(&original, &moved).map_err(|error| error.to_string())?;

    let error = revalidate_execution_root(&prepared.descriptor, nest.path())
        .err()
        .ok_or_else(|| "missing managed target should fail closed".to_string())?;
    assert!(error.contains("managed directory") || error.contains("inspect"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn symlinked_repository_bucket_cannot_escape_the_nest() -> Result<(), String> {
    use std::os::unix::fs::symlink;

    let repository = create_repository()?;
    let identity = inspect_repository(&path_to_string(&repository.root, "test repository")?)?
        .repository_identity;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let outside = tempfile::tempdir().map_err(|error| error.to_string())?;
    let worktrees = nest.path().join(WORKTREES_DIRECTORY);
    fs::create_dir(&worktrees).map_err(|error| error.to_string())?;
    symlink(outside.path(), worktrees.join(&identity)).map_err(|error| error.to_string())?;

    let result = prepare_execution_root(
        CodeWorktreePrepareInput {
            repository_root: path_to_string(&repository.root, "test repository")?,
            base_ref: "HEAD".to_string(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        nest.path(),
    );
    assert!(result.is_err());
    assert_eq!(
        fs::read_dir(outside.path())
            .map_err(|error| error.to_string())?
            .count(),
        0
    );
    Ok(())
}

#[test]
fn a_non_directory_worktrees_boundary_is_rejected() -> Result<(), String> {
    let repository = create_repository()?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    fs::write(nest.path().join(WORKTREES_DIRECTORY), "not a directory")
        .map_err(|error| error.to_string())?;

    assert!(prepare_execution_root(
        CodeWorktreePrepareInput {
            repository_root: path_to_string(&repository.root, "test repository")?,
            base_ref: "HEAD".to_string(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        nest.path(),
    )
    .is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn target_reservation_creates_an_empty_real_directory_and_rejects_replacement_symlinks(
) -> Result<(), String> {
    use std::os::unix::fs::symlink;

    let parent = tempfile::tempdir().map_err(|error| error.to_string())?;
    let parent = parent
        .path()
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let (_worktree_id, target) = reserve_worktree_target(&parent)?;

    let metadata = target
        .symlink_metadata()
        .map_err(|error| error.to_string())?;
    assert!(metadata.is_dir());
    assert!(!metadata.file_type().is_symlink());
    assert!(fs::read_dir(&target)
        .map_err(|error| error.to_string())?
        .next()
        .is_none());
    validate_reserved_worktree_target(&parent, &target)?;

    let outside = tempfile::tempdir().map_err(|error| error.to_string())?;
    fs::remove_dir(&target).map_err(|error| error.to_string())?;
    symlink(outside.path(), &target).map_err(|error| error.to_string())?;
    assert!(validate_reserved_worktree_target(&parent, &target).is_err());
    assert!(fs::read_dir(outside.path())
        .map_err(|error| error.to_string())?
        .next()
        .is_none());
    Ok(())
}

#[cfg(unix)]
#[test]
fn pinned_git_helper_envelope_is_bounded_and_strict() -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;

    let repository = create_repository()?;
    let target = repository
        .root
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let metadata = fs::metadata(&target).map_err(|error| error.to_string())?;
    let valid = PinnedGitEnvelope {
        version: PINNED_GIT_REQUEST_VERSION,
        target_device: metadata.dev(),
        target_inode: metadata.ino(),
        request: PinnedGitRequest::Checkout {
            git_executable: path_to_string(&git_executable()?, "git executable")?,
            base_commit: resolve_commit(&repository.root, "HEAD")?,
            disabled_filter_keys: Vec::new(),
            expected_target_path: path_to_string(&target, "test target")?,
        },
    };
    validate_pinned_git_envelope(&valid)?;

    let unknown = serde_json::to_value(&valid).map_err(|error| error.to_string())?;
    let mut unknown = unknown
        .as_object()
        .cloned()
        .ok_or_else(|| "test envelope was not an object".to_string())?;
    unknown.insert("unexpected".to_string(), serde_json::json!(true));
    assert!(
        serde_json::from_value::<PinnedGitEnvelope>(serde_json::Value::Object(unknown)).is_err()
    );

    for forbidden_operation in [
        "remove",
        "worktreeRemove",
        "delete",
        "destroy",
        "cleanup",
        "clean",
        "prune",
        "purge",
        "discard",
    ] {
        let mut removal = serde_json::to_value(&valid).map_err(|error| error.to_string())?;
        removal["request"]["operation"] = serde_json::json!(forbidden_operation);
        assert!(
            serde_json::from_value::<PinnedGitEnvelope>(removal).is_err(),
            "current pinned Git helper accepted forbidden operation {forbidden_operation}"
        );
    }

    let too_many_filters = PinnedGitEnvelope {
        request: PinnedGitRequest::Checkout {
            git_executable: path_to_string(&git_executable()?, "git executable")?,
            base_commit: resolve_commit(&repository.root, "HEAD")?,
            disabled_filter_keys: (0..=MAX_PINNED_GIT_FILTER_KEYS)
                .map(|index| format!("filter.test{index}.process"))
                .collect(),
            expected_target_path: path_to_string(&target, "test target")?,
        },
        ..valid
    };
    assert!(validate_pinned_git_envelope(&too_many_filters).is_err());

    let invalid_read_filter = PinnedGitEnvelope {
        request: PinnedGitRequest::ReadOnly {
            git_executable: path_to_string(&git_executable()?, "git executable")?,
            command: CodePinnedReadCommand::LocalConfig,
            disabled_filter_keys: vec!["filter.bad.command".to_string()],
            expected_target_path: path_to_string(&target, "test target")?,
        },
        ..too_many_filters
    };
    assert!(validate_pinned_git_envelope(&invalid_read_filter).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn pinned_target_swap_cannot_redirect_git_into_an_outside_directory() -> Result<(), String> {
    use std::os::unix::fs::symlink;

    let repository = create_repository()?;
    let repository_info = discover_repository(&repository.root)?;
    let base_commit = resolve_commit(&repository.root, "HEAD")?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let nest = nest
        .path()
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let worktrees = ensure_real_child_directory(&nest, WORKTREES_DIRECTORY)?;
    let repository_bucket = ensure_real_child_directory(&worktrees, &repository_info.identity)?;
    let worktree_id = Uuid::new_v4().to_string();
    let target = repository_bucket.join(&worktree_id);
    create_private_directory(&target).map_err(|error| error.to_string())?;
    let operation = prepare_pinned_git_operation(
        &target,
        PinnedGitRequest::WorktreeAdd {
            git_executable: path_to_string(&git_executable()?, "git executable")?,
            git_common_dir: path_to_string(&repository_info.common_dir, "Git common directory")?,
            base_commit,
            disabled_filter_keys: repository_filter_overrides(&repository.root)?,
            expected_target_path: path_to_string(&target, "managed worktree target")?,
        },
    )?;

    let outside = tempfile::tempdir().map_err(|error| error.to_string())?;
    let moved_target = outside.path().join("moved-reserved-target");
    fs::rename(&target, &moved_target).map_err(|error| error.to_string())?;
    symlink(outside.path(), &target).map_err(|error| error.to_string())?;

    assert!(run_git_with_pinned_operation(operation).is_err());
    assert!(!moved_target.join(".git").exists());
    assert!(validate_reserved_worktree_target(&repository_bucket, &target).is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn helper_uses_the_pinned_directory_if_the_name_changes_after_validation() -> Result<(), String> {
    use std::os::unix::fs::symlink;

    let repository = create_repository()?;
    let repository_info = discover_repository(&repository.root)?;
    let base_commit = resolve_commit(&repository.root, "HEAD")?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let nest = nest
        .path()
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let worktrees = ensure_real_child_directory(&nest, WORKTREES_DIRECTORY)?;
    let repository_bucket = ensure_real_child_directory(&worktrees, &repository_info.identity)?;
    let worktree_id = Uuid::new_v4().to_string();
    let target = repository_bucket.join(&worktree_id);
    create_private_directory(&target).map_err(|error| error.to_string())?;
    let operation = prepare_pinned_git_operation(
        &target,
        PinnedGitRequest::WorktreeAdd {
            git_executable: path_to_string(&git_executable()?, "git executable")?,
            git_common_dir: path_to_string(&repository_info.common_dir, "Git common directory")?,
            base_commit,
            disabled_filter_keys: repository_filter_overrides(&repository.root)?,
            expected_target_path: path_to_string(&target, "managed worktree target")?,
        },
    )?;
    verify_pinned_target_chain(&operation.request, &operation.directories)?;

    // Model a same-UID rename after the parent's pre-spawn validation.
    // The helper must enter the already-open target descriptor instead of
    // following the replacement symlink at the original pathname.
    let outside = tempfile::tempdir().map_err(|error| error.to_string())?;
    let moved_target = outside.path().join("moved-pinned-target");
    fs::rename(&target, &moved_target).map_err(|error| error.to_string())?;
    symlink(outside.path(), &target).map_err(|error| error.to_string())?;

    spawn_pinned_git_helper(
        &operation.request,
        &operation.directories,
        &operation.launch,
    )?;

    assert!(!outside.path().join(".git").exists());
    assert!(moved_target.join(".git").exists());
    assert!(verify_pinned_target_chain(&operation.request, &operation.directories).is_err());
    Ok(())
}
