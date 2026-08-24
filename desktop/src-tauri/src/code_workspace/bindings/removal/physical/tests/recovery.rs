use super::*;

#[test]
pub(super) fn crash_after_quarantine_recovers_same_removal_and_preserves_external_state(
) -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let sibling_before = snapshot_tree(&fixture.sibling_root)?;
    let transcript_before = fs::read(&fixture.transcript).map_err(|error| error.to_string())?;
    let mut fault = FailOnceAt {
        target: unix::FaultBoundary::Quarantined,
        tripped: false,
    };

    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut fault,
    )
    .expect_err("quarantine boundary fault should interrupt removal");
    assert!(error.contains("injected crash after Quarantined"));
    assert!(fault.tripped);

    let removing = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "crash did not preserve a removal journal".to_string())?;
    if !matches!(removing, CodeWorktreeRemovalRecord::Removing(_)) {
        return Err("crash rolled the sticky removal state back".to_string());
    }
    let removal_id = removing.authority().removal_id.clone();
    let quarantine = Path::new(&removing.authority().physical.managed_root_parent)
        .join(&removing.authority().physical.quarantine_name);
    assert!(fs::symlink_metadata(&fixture.managed_root).is_err());
    assert!(fs::symlink_metadata(&quarantine).is_ok());
    assert!(Path::new(&removing.authority().physical.git_admin_parent)
        .join(&removing.authority().physical.git_admin_entry)
        .is_dir());
    let proof_ref = format!("refs/schoolx/removal-claims/{removal_id}");
    assert_eq!(
        git_line(
            &fixture.source_root,
            &["show-ref", "--verify", "--hash", &proof_ref],
        )?,
        removing.authority().merge_proof.target_commit
    );

    let reopened = CodeThreadBindingStore::for_app_data(&fixture.app_data)?;
    unix::recover_all(&reopened, &fixture.nest_root, &mut unix::NoopFaultHook)?;
    let removed = reopened
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "recovery lost the removal tombstone".to_string())?;
    assert_eq!(removed.authority().removal_id, removal_id);
    assert_removed(&fixture, &removed)?;
    assert_eq!(snapshot_tree(&fixture.sibling_root)?, sibling_before);
    assert_eq!(
        fs::read(&fixture.transcript).map_err(|error| error.to_string())?,
        transcript_before
    );
    Ok(())
}

#[test]
pub(super) fn quarantined_non_prefix_external_deletion_is_sticky_without_additional_mutation(
) -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let mut hook = DeleteFutureRootEntryAtQuarantine {
        store: &fixture.store,
        lookup: &fixture.lookup,
        tripped: false,
    };
    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut hook,
    )
    .expect_err("quarantine hook should interrupt after its external deletion");
    assert!(error.contains("injected non-prefix manifest deletion"));
    assert!(hook.tripped);

    let removing = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "external deletion lost the removal journal".to_string())?;
    if !matches!(removing, CodeWorktreeRemovalRecord::Removing(_)) {
        return Err("external deletion did not leave a sticky Removing record".to_string());
    }
    let authority = removing.authority();
    let quarantine = Path::new(&authority.physical.managed_root_parent)
        .join(&authority.physical.quarantine_name);
    assert!(quarantine.is_dir());
    assert!(
        fs::symlink_metadata(quarantine.join(".git")).is_err(),
        "future manifest entry was not externally deleted"
    );
    assert!(
        quarantine.join("nested").join("tracked.txt").is_file(),
        "the deterministic first manifest entry was unexpectedly absent"
    );
    let before = snapshot_removing_retry_state(&fixture, &removing)?;

    let retry_error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut unix::NoopFaultHook,
    )
    .expect_err("non-prefix manifest deletion must reject sticky retry");
    assert!(
        retry_error.contains("deletion outside the known prefix"),
        "unexpected retry rejection: {retry_error}"
    );
    assert_eq!(
        fixture.store.lookup_worktree_removal(&fixture.lookup)?,
        Some(removing.clone()),
        "retry changed the sticky Removing journal"
    );
    assert_eq!(
        snapshot_removing_retry_state(&fixture, &removing)?,
        before,
        "rejected retry performed an additional mutation"
    );
    Ok(())
}

#[test]
pub(super) fn quarantined_original_path_replacement_is_preserved_without_tombstone(
) -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let sentinel = b"valuable replacement bytes\n".to_vec();
    let mut hook = InstallOriginalReplacementAtQuarantine {
        original: fixture.managed_root.clone(),
        sentinel: sentinel.clone(),
        tripped: false,
    };
    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut hook,
    )
    .expect_err("quarantine hook should interrupt after installing a replacement");
    assert!(error.contains("injected original-path replacement"));
    assert!(hook.tripped);

    let removing = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "replacement injection lost the removal journal".to_string())?;
    if !matches!(removing, CodeWorktreeRemovalRecord::Removing(_)) {
        return Err("replacement injection did not leave a sticky Removing record".to_string());
    }
    let authority = removing.authority();
    let quarantine = Path::new(&authority.physical.managed_root_parent)
        .join(&authority.physical.quarantine_name);
    assert!(quarantine.is_dir());
    assert_eq!(
        fs::read(fixture.managed_root.join("sentinel.txt")).map_err(|error| error.to_string())?,
        sentinel
    );
    let before = snapshot_removing_retry_state(&fixture, &removing)?;

    let retry_error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut unix::NoopFaultHook,
    )
    .expect_err("original-coordinate replacement must reject sticky retry");
    assert!(
        retry_error.contains("coordinates contain a replacement or ambiguous state"),
        "unexpected retry rejection: {retry_error}"
    );
    assert_eq!(
        fs::read(fixture.managed_root.join("sentinel.txt")).map_err(|error| error.to_string())?,
        sentinel,
        "retry changed the replacement sentinel"
    );
    assert_eq!(
        fixture.store.lookup_worktree_removal(&fixture.lookup)?,
        Some(removing.clone()),
        "retry created a Removed tombstone"
    );
    assert_eq!(
        snapshot_removing_retry_state(&fixture, &removing)?,
        before,
        "rejected replacement retry performed an additional mutation"
    );
    Ok(())
}

#[test]
pub(super) fn quarantined_sibling_move_and_replacement_are_preserved_sticky() -> Result<(), String>
{
    let fixture = prepare_fixture()?;
    let relocated_name = "relocated-exact-quarantine".to_string();
    let sentinel = b"valuable quarantine replacement bytes\n".to_vec();
    let mut hook = ReplaceQuarantineWithSiblingAtQuarantined {
        store: &fixture.store,
        lookup: &fixture.lookup,
        relocated_name: relocated_name.clone(),
        sentinel: sentinel.clone(),
        tripped: false,
    };
    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut hook,
    )
    .expect_err("quarantine hook should interrupt after installing its replacement");
    assert!(error.contains("injected quarantine sibling replacement"));
    assert!(hook.tripped);

    let removing = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "quarantine replacement lost the removal journal".to_string())?;
    if !matches!(removing, CodeWorktreeRemovalRecord::Removing(_)) {
        return Err("quarantine replacement did not leave a sticky Removing record".to_string());
    }
    let authority = removing.authority();
    let root_parent = Path::new(&authority.physical.managed_root_parent);
    let relocated = root_parent.join(&relocated_name);
    let replacement = root_parent.join(&authority.physical.quarantine_name);
    assert!(relocated.join(".git").is_file());
    assert!(relocated.join("nested").join("tracked.txt").is_file());
    assert_eq!(
        fs::read(replacement.join("sentinel.txt")).map_err(|error| error.to_string())?,
        sentinel
    );
    let before = snapshot_removing_retry_state(&fixture, &removing)?;

    let retry_error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut unix::NoopFaultHook,
    )
    .expect_err("quarantine-coordinate replacement must reject sticky retry");
    assert!(
        !retry_error.is_empty(),
        "quarantine replacement retry returned an empty error"
    );
    assert!(relocated.join(".git").is_file());
    assert_eq!(
        fs::read(replacement.join("sentinel.txt")).map_err(|error| error.to_string())?,
        sentinel,
        "retry changed the quarantine replacement"
    );
    assert_eq!(
        fixture.store.lookup_worktree_removal(&fixture.lookup)?,
        Some(removing.clone()),
        "retry changed the sticky Removing journal"
    );
    assert_eq!(
        snapshot_removing_retry_state(&fixture, &removing)?,
        before,
        "retry mutated the moved quarantine or its replacement"
    );
    Ok(())
}

#[test]
pub(super) fn root_deleted_git_admin_move_and_replacement_remain_cleanup_pending(
) -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let relocated_admin = fixture._directory.path().join("relocated-exact-git-admin");
    let sentinel = b"valuable Git-admin replacement bytes\n".to_vec();
    let mut hook = ReplaceGitAdminAtRootDeleted {
        store: &fixture.store,
        lookup: &fixture.lookup,
        relocated_admin: relocated_admin.clone(),
        sentinel: sentinel.clone(),
        tripped: false,
    };
    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut hook,
    )
    .expect_err("RootDeleted hook should interrupt after replacing Git-admin authority");
    assert!(error.contains("injected Git-admin replacement after RootDeleted"));
    assert!(hook.tripped);

    let removing = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "Git-admin replacement lost the removal journal".to_string())?;
    if !matches!(removing, CodeWorktreeRemovalRecord::Removing(_)) {
        return Err("Git-admin replacement did not leave a sticky Removing record".to_string());
    }
    let authority = removing.authority();
    let quarantine = Path::new(&authority.physical.managed_root_parent)
        .join(&authority.physical.quarantine_name);
    assert!(fs::symlink_metadata(&fixture.managed_root).is_err());
    assert!(fs::symlink_metadata(&quarantine).is_err());
    assert!(relocated_admin.join("HEAD").is_file());
    assert!(relocated_admin.join("index").is_file());
    let replacement =
        Path::new(&authority.physical.git_admin_parent).join(&authority.physical.git_admin_entry);
    assert_eq!(
        fs::read(replacement.join("sentinel.txt")).map_err(|error| error.to_string())?,
        sentinel
    );
    let sidecar = fixture
        .app_data
        .join("code")
        .join("removal-manifests-v1")
        .join(format!("{}.json", authority.physical_manifest_digest));
    let sidecar_before =
        fs::read(&sidecar).map_err(|error| format!("cleanup sidecar was missing: {error}"))?;
    let proof_ref = format!("refs/schoolx/removal-claims/{}", authority.removal_id);
    assert_eq!(
        git_line(
            &fixture.source_root,
            &["show-ref", "--verify", "--hash", &proof_ref],
        )?,
        authority.merge_proof.target_commit
    );
    let relocated_before = snapshot_tree(&relocated_admin)?;
    let before = snapshot_removing_retry_state(&fixture, &removing)?;

    let retry_error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut unix::NoopFaultHook,
    )
    .expect_err("Git-admin coordinate replacement must reject sticky retry");
    assert!(
        retry_error.contains("coordinates contain a replacement or ambiguous state"),
        "unexpected retry rejection: {retry_error}"
    );
    assert_eq!(snapshot_tree(&relocated_admin)?, relocated_before);
    assert_eq!(
        fs::read(replacement.join("sentinel.txt")).map_err(|error| error.to_string())?,
        sentinel,
        "retry changed the Git-admin replacement"
    );
    assert_eq!(
        fs::read(&sidecar).map_err(|error| error.to_string())?,
        sidecar_before,
        "retry cleaned or changed the removal sidecar"
    );
    assert_eq!(
        git_line(
            &fixture.source_root,
            &["show-ref", "--verify", "--hash", &proof_ref],
        )?,
        authority.merge_proof.target_commit,
        "retry cleaned or changed the removal proof ref"
    );
    assert_eq!(
        fixture.store.lookup_worktree_removal(&fixture.lookup)?,
        Some(removing.clone()),
        "retry changed the sticky Removing journal"
    );
    assert_eq!(
        snapshot_removing_retry_state(&fixture, &removing)?,
        before,
        "retry mutated Git-admin replacement or cleanup-pending state"
    );
    Ok(())
}
