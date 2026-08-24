use super::*;

#[cfg(unix)]
#[test]
fn thread_changes_reads_exact_local_checkout_without_mutating_it() -> Result<(), String> {
    use std::os::unix::fs::symlink;

    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let repository = create_test_repository()?;
    let literal_magic_path = ":(glob)*.txt";
    fs::write(
        repository.root.join(literal_magic_path),
        "literal baseline\n",
    )
    .map_err(|error| error.to_string())?;
    fs::write(repository.root.join("decoy.txt"), "decoy baseline\n")
        .map_err(|error| error.to_string())?;
    fs::write(repository.root.join("binary.dat"), [0_u8, 1, 2])
        .map_err(|error| error.to_string())?;
    fs::write(repository.root.join("deleted.txt"), "deleted baseline\n")
        .map_err(|error| error.to_string())?;
    fs::write(
        repository.root.join("type-change.txt"),
        "regular baseline\n",
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        repository.root.join(".gitattributes"),
        "filtered.txt filter=schoolx-phase1f\n",
    )
    .map_err(|error| error.to_string())?;
    fs::write(repository.root.join("filtered.txt"), "filtered baseline\n")
        .map_err(|error| error.to_string())?;
    test_git(&repository.root, &["add", "-A"])?;
    test_git(
        &repository.root,
        &[
            "-c",
            "user.name=SchoolX Test",
            "-c",
            "user.email=schoolx@example.invalid",
            "commit",
            "-m",
            "literal pathspec fixture",
        ],
    )?;
    let (scope, binding) = persisted_local_binding(&repository, "thread-changes")?;
    CodeThreadBindingStore::for_app_data(app_data.path())?.upsert(binding.clone())?;
    test_git(
        &repository.root,
        &[
            "config",
            "filter.schoolx-phase1f.clean",
            "sh -c 'touch filter-marker; cat'",
        ],
    )?;
    test_git(
        &repository.root,
        &["config", "filter.schoolx-phase1f.required", "true"],
    )?;

    fs::write(repository.root.join("README.md"), "phase 1f changed\n")
        .map_err(|error| error.to_string())?;
    fs::write(
        repository.root.join(literal_magic_path),
        "literal changed\n",
    )
    .map_err(|error| error.to_string())?;
    fs::write(repository.root.join("decoy.txt"), "decoy changed\n")
        .map_err(|error| error.to_string())?;
    fs::write(repository.root.join("binary.dat"), [0_u8, 1, 3])
        .map_err(|error| error.to_string())?;
    fs::remove_file(repository.root.join("deleted.txt")).map_err(|error| error.to_string())?;
    fs::remove_file(repository.root.join("type-change.txt")).map_err(|error| error.to_string())?;
    symlink("README.md", repository.root.join("type-change.txt"))
        .map_err(|error| error.to_string())?;
    fs::write(repository.root.join("filtered.txt"), "filtered changed\n")
        .map_err(|error| error.to_string())?;
    fs::write(repository.root.join("staged.txt"), "staged\n").map_err(|error| error.to_string())?;
    test_git(&repository.root, &["add", "staged.txt"])?;
    fs::write(repository.root.join("untracked.txt"), "one\ntwo\n")
        .map_err(|error| error.to_string())?;
    fs::write(repository.root.join("untracked.bin"), [0_u8, 4, 5])
        .map_err(|error| error.to_string())?;
    fs::write(repository.root.join("empty.txt"), b"").map_err(|error| error.to_string())?;
    let before = test_git(&repository.root, &["status", "--porcelain=v1", "-z"])?;
    let filter_marker = repository.root.join("filter-marker");
    if filter_marker.exists() {
        fs::remove_file(&filter_marker).map_err(|error| error.to_string())?;
    }
    let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;
    let limited = crate::commands::project_git_diff::current_changes_from_repo_with_limit(
        &repository.root,
        &auth,
        &binding.base_ref,
        &binding.repository_identity,
        3,
    )?;
    assert_eq!(limited.total_files, 11);
    assert_eq!(limited.files.len(), 3);
    assert!(limited.files_truncated);
    assert_eq!(
        limited
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec![":(glob)*.txt", "README.md", "binary.dat"]
    );
    assert_eq!(
        limited.additions,
        limited
            .files
            .iter()
            .map(|file| file.additions)
            .sum::<usize>()
    );
    assert_eq!(
        limited.deletions,
        limited
            .files
            .iter()
            .map(|file| file.deletions)
            .sum::<usize>()
    );

    let changes = thread_changes_native(
        CodeThreadChangesInput {
            scope: scope.clone(),
            thread_id: "thread-changes".to_string(),
        },
        app_data.path(),
        nest.path(),
        &auth,
    )?;

    assert_eq!(
        changes
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec![
            ":(glob)*.txt",
            "README.md",
            "binary.dat",
            "decoy.txt",
            "deleted.txt",
            "empty.txt",
            "filtered.txt",
            "staged.txt",
            "type-change.txt",
            "untracked.bin",
            "untracked.txt"
        ]
    );
    assert_eq!(changes.total_files, 11);
    assert!(!changes.files_truncated);
    let literal_patch = changes
        .files
        .iter()
        .find(|file| file.path == literal_magic_path)
        .map(|file| file.patch.as_str())
        .ok_or_else(|| "literal pathspec change was missing".to_string())?;
    assert!(literal_patch.contains("+literal changed"));
    assert!(!literal_patch.contains("+decoy changed"));
    assert!(!filter_marker.exists());
    assert!(changes
        .files
        .iter()
        .find(|file| file.path == "README.md")
        .is_some_and(|file| {
            file.status == CodeThreadChangeStatus::Modified
                && !file.binary
                && file.additions == 1
                && file.deletions == 1
        }));
    assert!(changes
        .files
        .iter()
        .find(|file| file.path == "staged.txt")
        .is_some_and(|file| file.status == CodeThreadChangeStatus::Added && !file.binary));
    assert!(changes
        .files
        .iter()
        .find(|file| file.path == "deleted.txt")
        .is_some_and(|file| file.status == CodeThreadChangeStatus::Deleted && !file.binary));
    assert!(changes
        .files
        .iter()
        .find(|file| file.path == "type-change.txt")
        .is_some_and(|file| {
            file.status == CodeThreadChangeStatus::TypeChanged && !file.binary
        }));
    assert!(changes
        .files
        .iter()
        .find(|file| file.path == "binary.dat")
        .is_some_and(|file| {
            file.status == CodeThreadChangeStatus::Modified
                && file.binary
                && file.additions == 0
                && file.deletions == 0
        }));
    assert!(changes
        .files
        .iter()
        .find(|file| file.path == "untracked.txt")
        .is_some_and(|file| {
            file.status == CodeThreadChangeStatus::Untracked
                && !file.binary
                && file.patch.contains("+one")
                && file.additions == 2
        }));
    assert!(changes
        .files
        .iter()
        .find(|file| file.path == "untracked.bin")
        .is_some_and(|file| {
            file.status == CodeThreadChangeStatus::Untracked
                && file.binary
                && file.additions == 0
                && file.deletions == 0
        }));
    assert!(changes
        .files
        .iter()
        .find(|file| file.path == "empty.txt")
        .is_some_and(|file| {
            file.status == CodeThreadChangeStatus::Untracked
                && !file.binary
                && file.additions == 0
                && file.deletions == 0
        }));
    assert_eq!(
        changes.additions,
        changes
            .files
            .iter()
            .map(|file| file.additions)
            .sum::<usize>()
    );
    assert_eq!(
        changes.deletions,
        changes
            .files
            .iter()
            .map(|file| file.deletions)
            .sum::<usize>()
    );
    assert_eq!(
        test_git(&repository.root, &["status", "--porcelain=v1", "-z"])?,
        before
    );

    let mut wrong_scope = scope.clone();
    wrong_scope.community_id = "other-community".to_string();
    assert!(thread_changes_native(
        CodeThreadChangesInput {
            scope: wrong_scope,
            thread_id: "thread-changes".to_string(),
        },
        app_data.path(),
        nest.path(),
        &auth,
    )
    .is_err());
    assert!(thread_changes_native(
        CodeThreadChangesInput {
            scope,
            thread_id: "other-thread".to_string(),
        },
        app_data.path(),
        nest.path(),
        &auth,
    )
    .is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn thread_changes_ignores_replace_refs_for_immutable_base() -> Result<(), String> {
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let repository = create_test_repository()?;
    let (scope, binding) = persisted_local_binding(&repository, "thread-replaced-base-object")?;
    let base_commit = binding.base_ref.clone();
    CodeThreadBindingStore::for_app_data(app_data.path())?.upsert(binding)?;

    fs::write(repository.root.join("README.md"), "replacement view\n")
        .map_err(|error| error.to_string())?;
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
            "replacement commit fixture",
        ],
    )?;
    let replacement_commit = test_git_line(&repository.root, &["rev-parse", "HEAD"])?;
    assert_ne!(replacement_commit, base_commit);

    test_git(&repository.root, &["checkout", "--detach", &base_commit])?;
    fs::write(repository.root.join("README.md"), "replacement view\n")
        .map_err(|error| error.to_string())?;
    test_git(
        &repository.root,
        &["replace", &base_commit, &replacement_commit],
    )?;

    let unprotected_diff = test_git(
        &repository.root,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            &base_commit,
            "--",
            "README.md",
        ],
    )?;
    assert!(
        unprotected_diff.is_empty(),
        "Git replacement fixture did not hide the immutable-base change"
    );
    let literal_diff = String::from_utf8(test_git(
        &repository.root,
        &[
            "--no-replace-objects",
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            &base_commit,
            "--",
            "README.md",
        ],
    )?)
    .map_err(|error| error.to_string())?;
    assert!(literal_diff.contains("-phase 1c"));
    assert!(literal_diff.contains("+replacement view"));

    let status_before = test_git(&repository.root, &["status", "--porcelain=v1", "-z"])?;
    let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;
    let changes = thread_changes_native(
        CodeThreadChangesInput {
            scope,
            thread_id: "thread-replaced-base-object".to_string(),
        },
        app_data.path(),
        nest.path(),
        &auth,
    )?;

    assert_eq!(changes.files.len(), 1);
    assert!(changes.files.first().is_some_and(|file| {
        file.path == "README.md"
            && file.additions == 1
            && file.deletions == 1
            && file.patch.contains("-phase 1c")
            && file.patch.contains("+replacement view")
    }));
    assert_eq!(changes.additions, 1);
    assert_eq!(changes.deletions, 1);
    assert_eq!(
        test_git(&repository.root, &["status", "--porcelain=v1", "-z"])?,
        status_before
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn thread_changes_reads_persisted_managed_worktree_from_immutable_base() -> Result<(), String> {
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let repository = create_test_repository()?;
    let repository_root = repository.root.to_string_lossy().into_owned();
    let inspected = preflight_execution_root(&repository_root, "main")?;
    let scope = phase1c_scope(inspected.repository_identity.clone());
    let base_commit = test_git_line(&repository.root, &["rev-parse", "main"])?;
    let binding_lock = Mutex::new(());

    let prepared = prepare_worktree_native(
        CodeWorktreePrepareCommandInput {
            scope: scope.clone(),
            repository_root,
            base_ref: "main".to_string(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        app_data.path(),
        nest.path(),
        &binding_lock,
    )?;
    assert_eq!(prepared.worktree.repository, inspected);
    assert_eq!(
        prepared.worktree.descriptor.execution_mode,
        CodeExecutionMode::Worktree
    );
    assert_eq!(
        prepared.worktree.descriptor.repository_identity,
        scope.repository_identity
    );
    assert_eq!(prepared.worktree.descriptor.base_ref, base_commit);
    let worktree_id = prepared
        .worktree
        .descriptor
        .worktree_id
        .as_deref()
        .ok_or_else(|| "managed preparation did not issue a worktree id".to_string())?;
    let execution_root = PathBuf::from(&prepared.worktree.descriptor.execution_root);
    let expected_root = nest
        .path()
        .canonicalize()
        .map_err(|error| error.to_string())?
        .join("WORKTREES")
        .join(&scope.repository_identity)
        .join(worktree_id);
    assert_eq!(execution_root, expected_root);
    let git_entry =
        fs::symlink_metadata(execution_root.join(".git")).map_err(|error| error.to_string())?;
    assert!(git_entry.is_file());
    assert!(!git_entry.file_type().is_symlink());

    let source_common_dir = PathBuf::from(test_git_line(
        &repository.root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?)
    .canonicalize()
    .map_err(|error| error.to_string())?;
    let managed_common_dir = PathBuf::from(test_git_line(
        &execution_root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?)
    .canonicalize()
    .map_err(|error| error.to_string())?;
    assert_eq!(managed_common_dir, source_common_dir);
    assert_eq!(
        crate::code_workspace::repository_identity(&managed_common_dir)?,
        scope.repository_identity
    );

    let thread_id = "thread-managed-changes";
    let binding = {
        let _guard = lock_bindings(&binding_lock)?;
        let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
        store.claim_preparation_for_start(&scope, &prepared.preparation_id, Vec::new())?;
        store.commit_preparation_binding(&scope, &prepared.preparation_id, thread_id)?
    };
    let reloaded_store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    let reloaded = reloaded_store.load()?;
    assert!(reloaded.preparations.is_empty());
    assert_eq!(reloaded.bindings, vec![binding.clone()]);
    let (_, target_ref) = reloaded_store
        .binding_merge_authority(&CodeThreadBindingLookupInput {
            scope: scope.clone(),
            codex_thread_id: thread_id.to_string(),
        })?
        .ok_or_else(|| "committed managed binding disappeared".to_string())?;
    assert_eq!(target_ref.as_deref(), Some("refs/heads/main"));
    assert_eq!(binding.execution_mode, CodeExecutionMode::Worktree);
    assert_eq!(binding.execution_root, execution_root.to_string_lossy());
    assert_eq!(binding.repository_identity, scope.repository_identity);
    assert_eq!(binding.base_ref, base_commit);
    assert_eq!(binding.worktree_id.as_deref(), Some(worktree_id));

    fs::write(repository.root.join("README.md"), "advanced main\n")
        .map_err(|error| error.to_string())?;
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
            "advance mutable main",
        ],
    )?;
    let advanced_main = test_git_line(&repository.root, &["rev-parse", "main"])?;
    assert_ne!(advanced_main, base_commit);

    fs::write(execution_root.join("README.md"), "advanced main\n")
        .map_err(|error| error.to_string())?;
    fs::write(
        execution_root.join("managed-only.txt"),
        "managed one\nmanaged two\n",
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        repository.root.join("source-decoy.txt"),
        "must not appear in managed Changes\n",
    )
    .map_err(|error| error.to_string())?;
    let source_status_before = test_git(&repository.root, &["status", "--porcelain=v1", "-z"])?;
    let managed_status_before = test_git(&execution_root, &["status", "--porcelain=v1", "-z"])?;
    let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;

    let changes = thread_changes_native(
        CodeThreadChangesInput {
            scope: scope.clone(),
            thread_id: thread_id.to_string(),
        },
        app_data.path(),
        nest.path(),
        &auth,
    )?;

    assert_eq!(
        changes
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["README.md", "managed-only.txt"]
    );
    assert!(changes
        .files
        .iter()
        .all(|file| file.path != "source-decoy.txt"));
    assert!(changes
        .files
        .iter()
        .find(|file| file.path == "README.md")
        .is_some_and(|file| {
            file.additions == 1
                && file.deletions == 1
                && file.patch.contains("-phase 1c")
                && file.patch.contains("+advanced main")
        }));
    assert!(changes
        .files
        .iter()
        .find(|file| file.path == "managed-only.txt")
        .is_some_and(|file| {
            file.additions == 2
                && file.deletions == 0
                && file.patch.contains("+managed one")
                && file.patch.contains("+managed two")
        }));
    assert_eq!(changes.additions, 3);
    assert_eq!(changes.deletions, 1);
    let source_status_after = test_git(&repository.root, &["status", "--porcelain=v1", "-z"])?;
    let managed_status_after = test_git(&execution_root, &["status", "--porcelain=v1", "-z"])?;
    assert_eq!(source_status_after, source_status_before);
    assert_eq!(managed_status_after, managed_status_before);

    let mut wrong_scope = scope;
    wrong_scope.project_dtag = "other-project".to_string();
    assert!(thread_changes_native(
        CodeThreadChangesInput {
            scope: wrong_scope,
            thread_id: thread_id.to_string(),
        },
        app_data.path(),
        nest.path(),
        &auth,
    )
    .is_err());
    assert!(thread_changes_native(
        CodeThreadChangesInput {
            scope: binding.scope(),
            thread_id: "other-thread".to_string(),
        },
        app_data.path(),
        nest.path(),
        &auth,
    )
    .is_err());
    Ok(())
}
