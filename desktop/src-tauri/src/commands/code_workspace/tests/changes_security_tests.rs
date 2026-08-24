use super::*;

#[cfg(unix)]
#[test]
fn thread_changes_root_swap_after_pin_fails_closed() -> Result<(), String> {
    let repository = create_test_repository()?;
    let (_, binding) = persisted_local_binding(&repository, "thread-swap-after-pin")?;
    fs::write(
        repository.root.join("README.md"),
        "original pinned change\n",
    )
    .map_err(|error| error.to_string())?;
    let moved = repository.root.with_file_name("moved-pinned-repository");
    let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;

    let result = crate::commands::project_git_diff::current_changes_from_repo_after_pin(
        &repository.root,
        &auth,
        &binding.base_ref,
        &binding.repository_identity,
        || {
            fs::rename(&repository.root, &moved).map_err(|error| error.to_string())?;
            fs::create_dir(&repository.root).map_err(|error| error.to_string())?;
            test_git(&repository.root, &["init", "--initial-branch=main"])?;
            fs::write(
                repository.root.join("replacement.txt"),
                "must not be read\n",
            )
            .map_err(|error| error.to_string())?;
            Ok(())
        },
    );

    let error = result
        .err()
        .ok_or_else(|| "root replacement unexpectedly produced a diff".to_string())?;
    assert!(error.contains("moved or was replaced"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn pinned_git_directory_rejects_same_base_dot_git_replacement() -> Result<(), String> {
    let repository = create_test_repository()?;
    let (_, binding) = persisted_local_binding(&repository, "thread-dot-git-swap")?;
    let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;
    let original_git = repository.root.join(".git-original");
    let replacement = repository.root.with_file_name("replacement-clone");

    let result = crate::commands::project_git_diff::current_changes_from_repo_after_pin(
        &repository.root,
        &auth,
        &binding.base_ref,
        &binding.repository_identity,
        || {
            fs::rename(repository.root.join(".git"), &original_git)
                .map_err(|error| error.to_string())?;
            let original_git_value = original_git.to_string_lossy().into_owned();
            let replacement_value = replacement.to_string_lossy().into_owned();
            test_git(
                repository
                    .root
                    .parent()
                    .ok_or_else(|| "test repository had no parent".to_string())?,
                &[
                    "clone",
                    "--no-hardlinks",
                    "--no-checkout",
                    &original_git_value,
                    &replacement_value,
                ],
            )?;
            fs::rename(replacement.join(".git"), repository.root.join(".git"))
                .map_err(|error| error.to_string())?;
            Ok(())
        },
    );

    let error = result
        .err()
        .ok_or_else(|| "same-base .git replacement was accepted".to_string())?;
    assert!(error.contains(".git entry"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn pinned_managed_gitfile_replacement_is_rejected() -> Result<(), String> {
    let repository = create_test_repository()?;
    let linked_root = repository.root.with_file_name("linked-worktree");
    let linked_value = linked_root.to_string_lossy().into_owned();
    test_git(
        &repository.root,
        &["worktree", "add", "--detach", &linked_value, "HEAD"],
    )?;
    let descriptor = preflight_execution_root(&linked_value, "HEAD")?;
    let head_output = test_git(&linked_root, &["rev-parse", "HEAD"])?;
    let base_ref = String::from_utf8(head_output)
        .map_err(|error| error.to_string())?
        .trim()
        .to_string();
    let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;
    let original_gitfile = linked_root.join(".git-original");

    let result = crate::commands::project_git_diff::current_changes_from_repo_after_pin(
        &linked_root,
        &auth,
        &base_ref,
        &descriptor.repository_identity,
        || {
            let contents = fs::read(linked_root.join(".git")).map_err(|error| error.to_string())?;
            fs::rename(linked_root.join(".git"), &original_gitfile)
                .map_err(|error| error.to_string())?;
            fs::write(linked_root.join(".git"), contents).map_err(|error| error.to_string())?;
            Ok(())
        },
    );

    let error = result
        .err()
        .ok_or_else(|| "managed gitfile replacement was accepted".to_string())?;
    assert!(error.contains(".git entry"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn thread_changes_classifies_a_real_index_conflict_as_unmerged() -> Result<(), String> {
    let repository = create_test_repository()?;
    fs::write(repository.root.join("conflict.txt"), "base\n").map_err(|error| error.to_string())?;
    test_git(&repository.root, &["add", "conflict.txt"])?;
    test_git(
        &repository.root,
        &[
            "-c",
            "user.name=SchoolX Test",
            "-c",
            "user.email=schoolx@example.invalid",
            "commit",
            "-m",
            "conflict base",
        ],
    )?;
    let (_, binding) = persisted_local_binding(&repository, "thread-conflict")?;

    test_git(&repository.root, &["checkout", "-b", "divergent"])?;
    fs::write(repository.root.join("conflict.txt"), "theirs\n")
        .map_err(|error| error.to_string())?;
    test_git(&repository.root, &["add", "conflict.txt"])?;
    test_git(
        &repository.root,
        &[
            "-c",
            "user.name=SchoolX Test",
            "-c",
            "user.email=schoolx@example.invalid",
            "commit",
            "-m",
            "divergent change",
        ],
    )?;

    test_git(&repository.root, &["checkout", "main"])?;
    fs::write(repository.root.join("conflict.txt"), "ours\n").map_err(|error| error.to_string())?;
    test_git(&repository.root, &["add", "conflict.txt"])?;
    test_git(
        &repository.root,
        &[
            "-c",
            "user.name=SchoolX Test",
            "-c",
            "user.email=schoolx@example.invalid",
            "commit",
            "-m",
            "main change",
        ],
    )?;
    assert!(test_git(&repository.root, &["merge", "divergent"]).is_err());

    let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;
    let changes = crate::commands::project_git_diff::current_changes_from_repo_with_limit(
        &repository.root,
        &auth,
        &binding.base_ref,
        &binding.repository_identity,
        10,
    )?;

    assert_eq!(changes.total_files, 1);
    assert!(!changes.files_truncated);
    assert!(changes.files.first().is_some_and(|file| {
        file.path == "conflict.txt"
            && file.status == crate::commands::project_git_diff::CurrentRepoChangeStatus::Unmerged
    }));
    assert!(String::from_utf8(test_git(
        &repository.root,
        &["diff", "--name-only", "--diff-filter=U"]
    )?)
    .map_err(|error| error.to_string())?
    .contains("conflict.txt"));
    Ok(())
}

#[cfg(unix)]
#[test]
fn thread_changes_retries_when_an_untracked_file_disappears_before_open() -> Result<(), String> {
    let repository = create_test_repository()?;
    let (_, binding) = persisted_local_binding(&repository, "thread-untracked-disappears")?;
    let disappearing = repository.root.join("disappearing.txt");
    fs::write(&disappearing, "temporary\n").map_err(|error| error.to_string())?;
    let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;
    let mut hook_calls = 0;

    let changes =
        crate::commands::project_git_diff::current_changes_from_repo_after_untracked_list(
            &repository.root,
            &auth,
            &binding.base_ref,
            &binding.repository_identity,
            || {
                hook_calls += 1;
                if hook_calls == 1 {
                    fs::remove_file(&disappearing).map_err(|error| error.to_string())?;
                }
                Ok(())
            },
        )?;

    assert_eq!(hook_calls, 2);
    assert_eq!(changes.total_files, 0);
    assert!(changes.files.is_empty());
    assert!(!changes.files_truncated);
    Ok(())
}

#[cfg(unix)]
#[test]
fn thread_changes_retries_one_drifted_inventory_once() -> Result<(), String> {
    let repository = create_test_repository()?;
    let (_, binding) = persisted_local_binding(&repository, "thread-transient-drift")?;
    let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;
    let mut hook_calls = 0;

    let changes =
        crate::commands::project_git_diff::current_changes_from_repo_after_untracked_list(
            &repository.root,
            &auth,
            &binding.base_ref,
            &binding.repository_identity,
            || {
                hook_calls += 1;
                if hook_calls == 1 {
                    fs::write(repository.root.join("appeared.txt"), "appeared\n")
                        .map_err(|error| error.to_string())?;
                }
                Ok(())
            },
        )?;

    assert_eq!(hook_calls, 2);
    assert_eq!(changes.total_files, 1);
    assert!(!changes.files_truncated);
    assert!(changes.files.first().is_some_and(|file| {
        file.path == "appeared.txt"
            && file.status == crate::commands::project_git_diff::CurrentRepoChangeStatus::Untracked
    }));
    Ok(())
}

#[cfg(unix)]
#[test]
fn thread_changes_reports_clear_error_after_repeated_inventory_drift() -> Result<(), String> {
    let repository = create_test_repository()?;
    let (_, binding) = persisted_local_binding(&repository, "thread-repeated-drift")?;
    let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;
    let mut hook_calls = 0;

    let error = crate::commands::project_git_diff::current_changes_from_repo_after_untracked_list(
        &repository.root,
        &auth,
        &binding.base_ref,
        &binding.repository_identity,
        || {
            hook_calls += 1;
            fs::write(
                repository.root.join(format!("drift-{hook_calls}.txt")),
                "drift\n",
            )
            .map_err(|error| error.to_string())
        },
    )
    .expect_err("two consecutive inventory drifts must fail closed");

    assert_eq!(hook_calls, 2);
    assert_eq!(
        error,
        "SchoolX Code Changes changed during inspection; retry after the workspace settles"
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn untracked_ancestor_symlink_swap_cannot_read_outside_root() -> Result<(), String> {
    use std::os::unix::fs::symlink;

    let repository = create_test_repository()?;
    let (_, binding) = persisted_local_binding(&repository, "thread-untracked-swap")?;
    let subdirectory = repository.root.join("sub");
    fs::create_dir(&subdirectory).map_err(|error| error.to_string())?;
    fs::write(subdirectory.join("file.txt"), "inside repository\n")
        .map_err(|error| error.to_string())?;
    let moved_subdirectory = repository.root.join("original-sub");
    let outside = tempfile::tempdir().map_err(|error| error.to_string())?;
    let sentinel = "outside-sentinel-must-not-be-read";
    fs::write(outside.path().join("file.txt"), sentinel).map_err(|error| error.to_string())?;
    let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;

    let result = crate::commands::project_git_diff::current_changes_from_repo_after_untracked_list(
        &repository.root,
        &auth,
        &binding.base_ref,
        &binding.repository_identity,
        || {
            fs::rename(&subdirectory, &moved_subdirectory).map_err(|error| error.to_string())?;
            symlink(outside.path(), &subdirectory).map_err(|error| error.to_string())?;
            Ok(())
        },
    );

    assert!(result.is_err());
    assert!(!format!("{result:?}").contains(sentinel));
    Ok(())
}

#[test]
fn thread_changes_rejects_missing_or_replaced_execution_root() -> Result<(), String> {
    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let repository = create_test_repository()?;
    let (scope, binding) = persisted_local_binding(&repository, "thread-replaced")?;
    CodeThreadBindingStore::for_app_data(app_data.path())?.upsert(binding)?;
    let auth = crate::commands::project_git_exec::build_test_git_auth_config()?;
    let original = repository.root.with_file_name("original-repository");
    fs::rename(&repository.root, &original).map_err(|error| error.to_string())?;

    let input = CodeThreadChangesInput {
        scope,
        thread_id: "thread-replaced".to_string(),
    };
    assert!(thread_changes_native(input.clone(), app_data.path(), nest.path(), &auth,).is_err());

    fs::create_dir(&repository.root).map_err(|error| error.to_string())?;
    test_git(&repository.root, &["init", "--initial-branch=main"])?;
    fs::write(repository.root.join("README.md"), "replacement\n")
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
            "replacement fixture",
        ],
    )?;
    assert!(thread_changes_native(input, app_data.path(), nest.path(), &auth).is_err());
    Ok(())
}
