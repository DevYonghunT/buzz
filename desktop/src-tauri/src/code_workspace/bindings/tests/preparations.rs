use std::collections::BTreeSet;

use super::*;

#[test]
fn preparation_journal_reloads_scopes_claims_and_commits_atomically() {
    let (directory, store) = store();
    let root = directory.path().join("local-preparation");
    fs::create_dir(&root).expect("local preparation root");
    let root = root.canonicalize().expect("canonical preparation root");
    let owner = scope("community-a", "project-a", 'a');
    let preparation_id = "67f11a1d-0274-4d40-9b0c-e406e51c64fb";
    let descriptor = local_descriptor(&root, 'a');

    let prepared = store
        .create_preparation(preparation_id.to_string(), owner.clone(), &descriptor)
        .expect("create preparation");
    assert_eq!(prepared.state, CodeThreadPreparationState::Prepared);

    let reloaded =
        CodeThreadBindingStore::for_app_data(directory.path()).expect("reopen preparation store");
    assert_eq!(
        reloaded
            .list_preparations(&owner)
            .expect("owner preparations"),
        vec![prepared.clone()]
    );
    assert!(reloaded
        .list_preparations(&scope("community-b", "project-a", 'a'))
        .expect("other-scope preparations")
        .is_empty());

    let claimed = reloaded
        .claim_preparation_for_start(&owner, preparation_id, vec!["thread-before".to_string()])
        .expect("claim preparation");
    assert_eq!(claimed.state, CodeThreadPreparationState::Starting);
    assert_eq!(
        claimed.recovery_thread_baseline,
        Some(vec!["thread-before".to_string()])
    );
    assert!(reloaded
        .claim_preparation_for_start(&owner, preparation_id, Vec::new())
        .is_err());

    let after_restart =
        CodeThreadBindingStore::for_app_data(directory.path()).expect("restart preparation store");
    assert_eq!(
        after_restart
            .starting_preparation(&owner, preparation_id)
            .expect("durable starting preparation")
            .recovery_thread_baseline,
        Some(vec!["thread-before".to_string()])
    );
    let binding = after_restart
        .commit_preparation_binding(&owner, preparation_id, "thread-recovered")
        .expect("commit recovered binding");
    assert_eq!(binding.codex_thread_id, "thread-recovered");
    let final_index = after_restart.load().expect("final binding index");
    assert!(final_index.preparations.is_empty());
    assert_eq!(final_index.bindings, vec![binding]);
}

#[test]
fn exact_unsent_start_snapshot_restores_and_reloads_as_prepared() {
    let (directory, store) = store();
    let root = directory.path().join("rollback-preparation");
    fs::create_dir(&root).expect("local preparation root");
    let root = root.canonicalize().expect("canonical preparation root");
    let owner = scope("community-a", "project-a", 'a');
    let preparation_id = "67f11a1d-0274-4d40-9b0c-e406e51c64fb";
    store
        .create_preparation(
            preparation_id.to_string(),
            owner.clone(),
            &local_descriptor(&root, 'a'),
        )
        .expect("create preparation");
    let claimed = store
        .claim_preparation_for_start(
            &owner,
            preparation_id,
            vec!["thread-z".to_string(), "thread-a".to_string()],
        )
        .expect("claim preparation");
    assert_eq!(
        claimed.recovery_thread_baseline,
        Some(vec!["thread-a".to_string(), "thread-z".to_string()])
    );

    let after_claim = CodeThreadBindingStore::for_app_data(directory.path())
        .expect("reopen claimed preparation store");
    assert_eq!(
        after_claim
            .starting_preparation(&owner, preparation_id)
            .expect("durable claimed preparation"),
        claimed
    );
    let restored = after_claim
        .restore_preparation_after_unsent_start(&claimed)
        .expect("restore definitely-unsent preparation");
    assert_eq!(restored.state, CodeThreadPreparationState::Prepared);
    assert_eq!(restored.recovery_thread_baseline, None);

    let after_restore = CodeThreadBindingStore::for_app_data(directory.path())
        .expect("reopen restored preparation store");
    assert_eq!(
        after_restore
            .prepared_preparation(&owner, preparation_id)
            .expect("durable restored preparation"),
        restored
    );
    assert!(after_restore
        .starting_preparation(&owner, preparation_id)
        .is_err());
}

#[test]
fn forged_unsent_start_snapshot_is_rejected_without_changing_starting_record() {
    let (directory, store) = store();
    let root = directory.path().join("forged-rollback-preparation");
    fs::create_dir(&root).expect("local preparation root");
    let root = root.canonicalize().expect("canonical preparation root");
    let owner = scope("community-a", "project-a", 'a');
    let preparation_id = "67f11a1d-0274-4d40-9b0c-e406e51c64fb";
    store
        .create_preparation(
            preparation_id.to_string(),
            owner.clone(),
            &local_descriptor(&root, 'a'),
        )
        .expect("create preparation");
    let claimed = store
        .claim_preparation_for_start(&owner, preparation_id, vec!["thread-before".to_string()])
        .expect("claim preparation");
    let mut forged = claimed.clone();
    forged.recovery_thread_baseline = Some(vec!["thread-forged".to_string()]);

    let error = store
        .restore_preparation_after_unsent_start(&forged)
        .expect_err("forged claimed snapshot must not roll back");
    assert!(
        error.contains("changed after claim"),
        "unexpected rollback error: {error}"
    );

    let reloaded = CodeThreadBindingStore::for_app_data(directory.path())
        .expect("reopen rejected rollback store");
    assert_eq!(
        reloaded
            .starting_preparation(&owner, preparation_id)
            .expect("unchanged starting preparation"),
        claimed
    );
    assert!(reloaded
        .prepared_preparation(&owner, preparation_id)
        .is_err());
}

#[test]
fn managed_preparation_reserves_its_worktree_before_thread_start() {
    let (directory, store) = store();
    let owner = scope("community", "project", 'a');
    let root = directory
        .path()
        .canonicalize()
        .expect("canonical app data")
        .join("managed-preparation");
    fs::create_dir(&root).expect("managed preparation root");
    let descriptor = CodeWorktreeDescriptor {
        execution_mode: CodeExecutionMode::Worktree,
        repository_identity: owner.repository_identity.clone(),
        execution_root: root.to_string_lossy().into_owned(),
        base_ref: "0123456789abcdef0123456789abcdef01234567".to_string(),
        worktree_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
    };
    store
        .create_preparation(
            "67f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
            owner.clone(),
            &descriptor,
        )
        .expect("managed preparation");

    assert!(store
        .create_preparation(
            "77f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
            owner.clone(),
            &descriptor,
        )
        .is_err());
    assert!(store
        .upsert(binding(
            &root,
            owner,
            "thread-duplicate",
            CodeExecutionMode::Worktree,
            Some("11111111-1111-4111-8111-111111111111"),
        ))
        .is_err());
}

#[test]
fn fork_preparation_binds_one_managed_source_and_rolls_back_exactly() {
    let (directory, store) = store();
    let owner = scope("community", "project", 'a');
    let source_root = directory.path().join("source-worktree");
    let destination_root = directory.path().join("destination-worktree");
    fs::create_dir(&source_root).expect("source root");
    fs::create_dir(&destination_root).expect("destination root");
    let source_root = source_root.canonicalize().expect("canonical source root");
    let destination_root = destination_root
        .canonicalize()
        .expect("canonical destination root");
    store
        .upsert(binding(
            &source_root,
            owner.clone(),
            "thread-source",
            CodeExecutionMode::Worktree,
            Some("11111111-1111-4111-8111-111111111111"),
        ))
        .expect("source binding");
    let descriptor = CodeWorktreeDescriptor {
        execution_mode: CodeExecutionMode::Worktree,
        repository_identity: owner.repository_identity.clone(),
        execution_root: destination_root.to_string_lossy().into_owned(),
        base_ref: "0123456789abcdef0123456789abcdef01234567".to_string(),
        worktree_id: Some("22222222-2222-4222-8222-222222222222".to_string()),
    };
    let preparation_id = "67f11a1d-0274-4d40-9b0c-e406e51c64fb";
    let prepared = store
        .create_fork_preparation(
            preparation_id.to_string(),
            owner.clone(),
            "thread-source".to_string(),
            &descriptor,
        )
        .expect("fork preparation");
    assert_eq!(prepared.operation, CodeThreadPreparationOperation::Fork);
    assert_eq!(prepared.source_thread_id.as_deref(), Some("thread-source"));
    assert!(prepared.merge_target_ref.is_none());
    assert!(store
        .ensure_fork_source_available(&owner, "thread-source")
        .is_err());
    assert!(store
        .create_fork_preparation(
            "77f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
            owner.clone(),
            "thread-source".to_string(),
            &CodeWorktreeDescriptor {
                worktree_id: Some("33333333-3333-4333-8333-333333333333".to_string()),
                ..descriptor.clone()
            },
        )
        .is_err());

    let claimed = store
        .claim_preparation_for_fork(
            &owner,
            preparation_id,
            "thread-source",
            vec!["thread-z".to_string(), "thread-a".to_string()],
        )
        .expect("claim fork");
    assert_eq!(
        claimed.recovery_thread_baseline,
        Some(vec!["thread-a".to_string(), "thread-z".to_string()])
    );
    let restored = store
        .restore_preparation_after_unsent_fork(&claimed)
        .expect("restore exact fork claim");
    assert_eq!(restored.state, CodeThreadPreparationState::Prepared);
    assert!(restored.recovery_thread_baseline.is_none());

    let claimed = store
        .claim_preparation_for_fork(&owner, preparation_id, "thread-source", Vec::new())
        .expect("reclaim fork");
    assert_eq!(claimed.state, CodeThreadPreparationState::Starting);
    let child = store
        .commit_preparation_binding(&owner, preparation_id, "thread-child")
        .expect("commit fork child");
    assert_eq!(child.codex_thread_id, "thread-child");
    assert_eq!(child.execution_root, descriptor.execution_root);
    let final_index = store.load().expect("final fork index");
    assert!(final_index.preparations.is_empty());
    assert_eq!(final_index.bindings.len(), 2);

    let mut malformed = final_index;
    malformed.preparations.push(CodeThreadPreparation {
        preparation_id: "87f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
        community_id: owner.community_id,
        project_dtag: owner.project_dtag,
        repository_identity: owner.repository_identity,
        execution_mode: CodeExecutionMode::Worktree,
        execution_root: destination_root.to_string_lossy().into_owned(),
        base_ref: descriptor.base_ref,
        worktree_id: Some("44444444-4444-4444-8444-444444444444".to_string()),
        operation: CodeThreadPreparationOperation::Fork,
        source_thread_id: Some("thread-missing".to_string()),
        state: CodeThreadPreparationState::Prepared,
        recovery_thread_baseline: None,
        merge_target_ref: None,
    });
    assert!(malformed.validate().is_err());
}

#[test]
fn merge_target_authority_is_native_only_and_moves_or_copies_atomically() {
    let (directory, store) = store();
    let owner = scope("community", "project", 'a');
    let source_root = directory.path().join("authority-source");
    let fork_root = directory.path().join("authority-fork");
    fs::create_dir(&source_root).expect("source root");
    fs::create_dir(&fork_root).expect("fork root");
    let source_root = source_root.canonicalize().expect("canonical source root");
    let fork_root = fork_root.canonicalize().expect("canonical fork root");
    let source_descriptor = CodeWorktreeDescriptor {
        execution_mode: CodeExecutionMode::Worktree,
        repository_identity: owner.repository_identity.clone(),
        execution_root: source_root.to_string_lossy().into_owned(),
        base_ref: "0123456789abcdef0123456789abcdef01234567".to_string(),
        worktree_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
    };
    let source_preparation = "11111111-1111-4111-8111-111111111112";
    let prepared = store
        .create_preparation_with_merge_target(
            source_preparation.to_string(),
            owner.clone(),
            &source_descriptor,
            Some("refs/heads/main".to_string()),
        )
        .expect("native-authorized preparation");
    assert_eq!(
        prepared.merge_target_ref.as_deref(),
        Some("refs/heads/main")
    );
    let public = store
        .list_preparations(&owner)
        .expect("public preparations");
    assert!(public[0].merge_target_ref.is_none());
    assert!(!serde_json::to_value(&public[0])
        .expect("serialize public preparation")
        .as_object()
        .expect("preparation object")
        .contains_key("mergeTargetRef"));

    let claimed = store
        .claim_preparation_for_start(&owner, source_preparation, Vec::new())
        .expect("claim source");
    assert_eq!(claimed.merge_target_ref.as_deref(), Some("refs/heads/main"));
    let restored = store
        .restore_preparation_after_unsent_start(&claimed)
        .expect("restore source");
    assert_eq!(
        restored.merge_target_ref.as_deref(),
        Some("refs/heads/main")
    );
    store
        .claim_preparation_for_start(&owner, source_preparation, Vec::new())
        .expect("reclaim source");
    let source_binding = store
        .commit_preparation_binding(&owner, source_preparation, "thread-source")
        .expect("commit source");
    assert_eq!(
        serde_json::to_value(&source_binding)
            .expect("serialize binding")
            .as_object()
            .map(|object| object.len()),
        Some(8)
    );
    let source_lookup = CodeThreadBindingLookupInput {
        scope: owner.clone(),
        codex_thread_id: "thread-source".to_string(),
    };
    let (_, target_ref) = store
        .binding_merge_authority(&source_lookup)
        .expect("binding authority")
        .expect("source binding");
    assert_eq!(target_ref.as_deref(), Some("refs/heads/main"));
    let valid_index = store.load().expect("authority index");
    assert_eq!(valid_index.merge_targets.len(), 1);
    let persisted = fs::read(store.store_path()).expect("read authority index");
    let persisted: serde_json::Value =
        serde_json::from_slice(&persisted).expect("parse authority index");
    assert_eq!(persisted["version"], serde_json::json!(4));
    assert_eq!(persisted["removals"], serde_json::json!([]));
    let top_level_keys = persisted
        .as_object()
        .expect("binding index object")
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        top_level_keys,
        [
            "bindings",
            "lifecycles",
            "mergeTargets",
            "preparations",
            "removals",
            "version",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>()
    );
    let target_keys = persisted["mergeTargets"][0]
        .as_object()
        .expect("merge-target object")
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        target_keys,
        [
            "codexThreadId",
            "communityId",
            "projectDtag",
            "repositoryIdentity",
            "targetRef",
            "worktreeId",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>()
    );
    let mut duplicate = valid_index.clone();
    let duplicate_authority = duplicate.merge_targets[0].clone();
    duplicate.merge_targets.push(duplicate_authority);
    assert!(duplicate.validate().is_err());
    let mut orphan = valid_index.clone();
    orphan.merge_targets[0].codex_thread_id = "thread-orphan".to_string();
    assert!(orphan.validate().is_err());
    let mut wrong_worktree = valid_index;
    wrong_worktree.merge_targets[0].worktree_id =
        "33333333-3333-4333-8333-333333333333".to_string();
    assert!(wrong_worktree.validate().is_err());

    let fork_descriptor = CodeWorktreeDescriptor {
        execution_mode: CodeExecutionMode::Worktree,
        repository_identity: owner.repository_identity.clone(),
        execution_root: fork_root.to_string_lossy().into_owned(),
        base_ref: source_descriptor.base_ref,
        worktree_id: Some("22222222-2222-4222-8222-222222222222".to_string()),
    };
    let fork = store
        .create_fork_preparation(
            "22222222-2222-4222-8222-222222222223".to_string(),
            owner,
            "thread-source".to_string(),
            &fork_descriptor,
        )
        .expect("fork preparation");
    assert_eq!(fork.merge_target_ref.as_deref(), Some("refs/heads/main"));
}

#[test]
fn merge_target_ref_validator_rejects_non_local_and_revision_syntax() {
    assert!(validate_direct_local_branch_ref("refs/heads/main").is_ok());
    assert!(validate_direct_local_branch_ref("refs/heads/team/topic").is_ok());
    for invalid in [
        "HEAD",
        "main",
        "refs/tags/main",
        "refs/remotes/origin/main",
        "refs/heads/main~1",
        "refs/heads/main@{1}",
        "refs/heads/.hidden",
        "refs/heads/main.lock",
        "refs/heads/team//topic",
    ] {
        assert!(
            validate_direct_local_branch_ref(invalid).is_err(),
            "unexpected valid merge target: {invalid}"
        );
    }
}
