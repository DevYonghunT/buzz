use super::*;

#[cfg(unix)]
#[test]
fn merge_proof_distinguishes_authorized_ancestry_and_is_zero_mutation() -> Result<(), String> {
    let repository = create_repository()?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let prepared = prepare_execution_root_with_merge_target(
        CodeWorktreePrepareInput {
            repository_root: path_to_string(&repository.root, "test repository")?,
            base_ref: "HEAD".to_string(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        nest.path(),
    )?;
    let descriptor = &prepared.worktree.descriptor;
    let target_ref = prepared
        .merge_target_ref
        .as_deref()
        .ok_or_else(|| "expected merge target".to_string())?;
    let managed_root = Path::new(&descriptor.execution_root);
    let initial = descriptor.base_ref.clone();
    let before = proof_observation(&repository.root, managed_root)?;
    let initial_proof = prove_direct_local_ancestry_before(
        descriptor,
        nest.path(),
        target_ref,
        Instant::now() + Duration::from_secs(30),
    )?;
    let CodeMergeProofOutcome::Proven(receipt) = initial_proof else {
        return Err("H == T was not proven".to_string());
    };
    assert_eq!(receipt.head_commit, initial);
    assert_eq!(receipt.target_commit, initial);
    assert_eq!(receipt.target_ref, "refs/heads/main");
    assert_eq!(receipt.repository_identity, descriptor.repository_identity);
    assert_eq!(
        receipt.worktree_id,
        descriptor.worktree_id.clone().unwrap_or_default()
    );
    assert_eq!(proof_observation(&repository.root, managed_root)?, before);

    let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
    let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
    let scope = crate::code_workspace::CodeThreadBindingScope {
        community_id: "community".to_string(),
        project_dtag: "project".to_string(),
        repository_identity: descriptor.repository_identity.clone(),
    };
    let preparation_id = "11111111-1111-4111-8111-111111111111";
    store.create_preparation_with_merge_target(
        preparation_id.to_string(),
        scope.clone(),
        descriptor,
        Some(target_ref.to_string()),
    )?;
    store.claim_preparation_for_start(&scope, preparation_id, Vec::new())?;
    store.commit_preparation_binding(&scope, preparation_id, "thread-proof")?;
    let store_before = fs::read(store.store_path()).map_err(|error| error.to_string())?;
    let native = prove_binding_merge_target_before(
        &store,
        &CodeThreadBindingLookupInput {
            scope,
            codex_thread_id: "thread-proof".to_string(),
        },
        nest.path(),
        Instant::now() + Duration::from_secs(30),
    )?;
    assert!(matches!(native, Some(CodeMergeProofOutcome::Proven(_))));
    assert_eq!(
        fs::read(store.store_path()).map_err(|error| error.to_string())?,
        store_before
    );

    let task_head = test_commit_file(managed_root, "task.txt", "task\n", "task")?;
    assert_eq!(
        prove_direct_local_ancestry_before(
            descriptor,
            nest.path(),
            target_ref,
            Instant::now() + Duration::from_secs(30),
        )?,
        CodeMergeProofOutcome::NotMerged
    );
    test_git(
        &repository.root,
        &["update-ref", "refs/heads/other", &task_head],
    )?;
    assert_eq!(
        prove_direct_local_ancestry_before(
            descriptor,
            nest.path(),
            target_ref,
            Instant::now() + Duration::from_secs(30),
        )?,
        CodeMergeProofOutcome::NotMerged
    );

    test_git(
        &repository.root,
        &[
            "-c",
            "user.name=SchoolX Test",
            "-c",
            "user.email=schoolx@example.invalid",
            "merge",
            "--no-ff",
            "-m",
            "merge task",
            &task_head,
        ],
    )?;
    let merged_target = resolve_commit(&repository.root, "refs/heads/main")?;
    let before_merged_proof = proof_observation(&repository.root, managed_root)?;
    let merged = prove_direct_local_ancestry_before(
        descriptor,
        nest.path(),
        target_ref,
        Instant::now() + Duration::from_secs(30),
    )?;
    let CodeMergeProofOutcome::Proven(receipt) = merged else {
        return Err("merge commit ancestry was not proven".to_string());
    };
    assert_eq!(receipt.head_commit, task_head);
    assert_eq!(receipt.target_commit, merged_target);
    assert_eq!(
        proof_observation(&repository.root, managed_root)?,
        before_merged_proof
    );
    assert!(!repository.root.join(".git/index.lock").exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn merge_proof_rejects_squash_grafts_deadline_and_snapshot_drift() -> Result<(), String> {
    let repository = create_repository()?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let prepared = prepare_execution_root_with_merge_target(
        CodeWorktreePrepareInput {
            repository_root: path_to_string(&repository.root, "test repository")?,
            base_ref: "HEAD".to_string(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        nest.path(),
    )?;
    let descriptor = &prepared.worktree.descriptor;
    let managed_root = Path::new(&descriptor.execution_root);
    let task_head = test_commit_file(managed_root, "squash.txt", "task\n", "task")?;
    test_git(&repository.root, &["merge", "--squash", &task_head])?;
    test_git(
        &repository.root,
        &[
            "-c",
            "user.name=SchoolX Test",
            "-c",
            "user.email=schoolx@example.invalid",
            "commit",
            "-m",
            "squash task",
        ],
    )?;
    assert_eq!(
        prove_direct_local_ancestry_before(
            descriptor,
            nest.path(),
            "refs/heads/main",
            Instant::now() + Duration::from_secs(30),
        )?,
        CodeMergeProofOutcome::NotMerged
    );

    let before_deadline = proof_observation(&repository.root, managed_root)?;
    assert!(prove_direct_local_ancestry_before(
        descriptor,
        nest.path(),
        "refs/heads/main",
        Instant::now(),
    )
    .is_err());
    assert_eq!(
        proof_observation(&repository.root, managed_root)?,
        before_deadline
    );

    let previous_target = resolve_commit(&repository.root, "refs/heads/main")?;
    let drift = prove_direct_local_ancestry_with_hook(
        descriptor,
        nest.path(),
        "refs/heads/main",
        Instant::now() + Duration::from_secs(30),
        || {
            test_git(
                &repository.root,
                &[
                    "update-ref",
                    "refs/heads/main",
                    &task_head,
                    &previous_target,
                ],
            )?;
            Ok(())
        },
    );
    assert!(drift.is_err());
    let head_drift = prove_direct_local_ancestry_with_hook(
        descriptor,
        nest.path(),
        "refs/heads/main",
        Instant::now() + Duration::from_secs(30),
        || {
            test_git(
                managed_root,
                &["update-ref", "HEAD", descriptor.base_ref.as_str()],
            )?;
            Ok(())
        },
    );
    assert!(head_drift.is_err());

    let second_repository = create_repository()?;
    let second_nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let second = prepare_execution_root_with_merge_target(
        CodeWorktreePrepareInput {
            repository_root: path_to_string(&second_repository.root, "test repository")?,
            base_ref: "HEAD".to_string(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        second_nest.path(),
    )?;
    let common_dir = discover_repository(&second_repository.root)?.common_dir;
    fs::create_dir_all(common_dir.join("info")).map_err(|error| error.to_string())?;
    fs::write(common_dir.join("info/grafts"), b"forged ancestry\n")
        .map_err(|error| error.to_string())?;
    assert!(prove_direct_local_ancestry_before(
        &second.worktree.descriptor,
        second_nest.path(),
        "refs/heads/main",
        Instant::now() + Duration::from_secs(30),
    )
    .is_err());
    Ok(())
}

#[cfg(unix)]
#[test]
fn merge_proof_ignores_replace_only_ancestry_and_rejects_missing_target() -> Result<(), String> {
    let repository = create_repository()?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let prepared = prepare_execution_root_with_merge_target(
        CodeWorktreePrepareInput {
            repository_root: path_to_string(&repository.root, "test repository")?,
            base_ref: "HEAD".to_string(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        nest.path(),
    )?;
    let descriptor = &prepared.worktree.descriptor;
    let managed_root = Path::new(&descriptor.execution_root);
    let task_head = test_commit_file(managed_root, "replace.txt", "task\n", "task")?;
    let target = resolve_commit(&repository.root, "refs/heads/main")?;
    let tree = test_line(&test_git(
        &repository.root,
        &["show", "-s", "--format=%T", &target],
    )?);
    let replacement = test_line(&test_git(
        &repository.root,
        &[
            "-c",
            "user.name=SchoolX Test",
            "-c",
            "user.email=schoolx@example.invalid",
            "commit-tree",
            &tree,
            "-p",
            &task_head,
            "-m",
            "replacement ancestry",
        ],
    )?);
    test_git(&repository.root, &["replace", &target, &replacement])?;
    assert!(test_git(
        &repository.root,
        &["merge-base", "--is-ancestor", &task_head, &target],
    )
    .is_ok());

    let before = proof_observation(&repository.root, managed_root)?;
    assert_eq!(
        prove_direct_local_ancestry_before(
            descriptor,
            nest.path(),
            "refs/heads/main",
            Instant::now() + Duration::from_secs(30),
        )?,
        CodeMergeProofOutcome::NotMerged
    );
    assert_eq!(proof_observation(&repository.root, managed_root)?, before);

    test_git(&repository.root, &["update-ref", "-d", "refs/heads/main"])?;
    assert!(prove_direct_local_ancestry_before(
        descriptor,
        nest.path(),
        "refs/heads/main",
        Instant::now() + Duration::from_secs(30),
    )
    .is_err());

    let common_dir = discover_repository(&repository.root)?.common_dir;
    let target_ref_path = common_dir.join("refs/heads/main");
    let missing_commit = "f".repeat(40);
    fs::write(&target_ref_path, format!("{missing_commit}\n"))
        .map_err(|error| error.to_string())?;
    let missing_object = common_dir
        .join("objects")
        .join(&missing_commit[..2])
        .join(&missing_commit[2..]);
    assert!(!missing_object.exists());
    let before_ref = fs::read(&target_ref_path).map_err(|error| error.to_string())?;
    let before_gitfile = fs::read(managed_root.join(".git")).map_err(|error| error.to_string())?;
    let before_head = test_git(managed_root, &["rev-parse", "HEAD"])?;
    assert!(prove_direct_local_ancestry_before(
        descriptor,
        nest.path(),
        "refs/heads/main",
        Instant::now() + Duration::from_secs(30),
    )
    .is_err());
    assert_eq!(
        fs::read(&target_ref_path).map_err(|error| error.to_string())?,
        before_ref
    );
    assert_eq!(
        fs::read(managed_root.join(".git")).map_err(|error| error.to_string())?,
        before_gitfile
    );
    assert_eq!(test_git(managed_root, &["rev-parse", "HEAD"])?, before_head);
    assert!(!missing_object.exists());
    Ok(())
}

#[cfg(unix)]
#[test]
fn merge_proof_rejects_cherry_pick_equivalence() -> Result<(), String> {
    let repository = create_repository()?;
    let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
    let prepared = prepare_execution_root_with_merge_target(
        CodeWorktreePrepareInput {
            repository_root: path_to_string(&repository.root, "test repository")?,
            base_ref: "HEAD".to_string(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        nest.path(),
    )?;
    let descriptor = &prepared.worktree.descriptor;
    let managed_root = Path::new(&descriptor.execution_root);
    let task_head = test_commit_file(managed_root, "picked.txt", "task\n", "task")?;
    test_commit_file(
        &repository.root,
        "main-only.txt",
        "main\n",
        "advance target before cherry-pick",
    )?;
    test_git(
        &repository.root,
        &[
            "-c",
            "user.name=SchoolX Test",
            "-c",
            "user.email=schoolx@example.invalid",
            "cherry-pick",
            &task_head,
        ],
    )?;
    assert_eq!(
        prove_direct_local_ancestry_before(
            descriptor,
            nest.path(),
            "refs/heads/main",
            Instant::now() + Duration::from_secs(30),
        )?,
        CodeMergeProofOutcome::NotMerged
    );
    Ok(())
}
