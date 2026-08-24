use super::*;

#[test]
pub(super) fn finalized_sidecar_replacement_preserves_tombstone_and_proof_ref() -> Result<(), String>
{
    let fixture = prepare_fixture()?;
    let relocated_sidecar = fixture
        ._directory
        .path()
        .join("relocated-exact-sidecar.json");
    let sentinel = b"valuable replacement sidecar bytes\n".to_vec();
    let mut hook = ReplaceSidecarAtFinalized {
        store: &fixture.store,
        lookup: &fixture.lookup,
        relocated_sidecar: relocated_sidecar.clone(),
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
    .expect_err("replacement sidecar must stop finalized cleanup");
    assert!(hook.tripped);
    assert!(
        error.contains("manifest sidecar"),
        "unexpected sidecar replacement rejection: {error}"
    );

    let removed = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "sidecar replacement lost the removal tombstone".to_string())?;
    if !matches!(removed, CodeWorktreeRemovalRecord::Removed(_)) {
        return Err("sidecar replacement did not preserve a Removed tombstone".to_string());
    }
    let authority = removed.authority();
    let sidecar = fixture
        .app_data
        .join("code")
        .join("removal-manifests-v1")
        .join(format!("{}.json", authority.physical_manifest_digest));
    assert_eq!(
        fs::read(&sidecar).map_err(|error| error.to_string())?,
        sentinel,
        "cleanup changed the replacement sidecar"
    );
    let exact_sidecar_before = fs::read(&relocated_sidecar)
        .map_err(|error| format!("relocated exact sidecar was not preserved: {error}"))?;
    let proof_ref = format!("refs/schoolx/removal-claims/{}", authority.removal_id);
    assert_eq!(
        git_line(
            &fixture.source_root,
            &["show-ref", "--verify", "--hash", &proof_ref],
        )?,
        authority.merge_proof.target_commit,
        "cleanup removed the proof ref despite a replacement sidecar"
    );

    let reopened = CodeThreadBindingStore::for_app_data(&fixture.app_data)?;
    let retry_error = unix::recover_all(&reopened, &fixture.nest_root, &mut unix::NoopFaultHook)
        .expect_err("replacement sidecar must keep cleanup fail closed");
    assert!(
        retry_error.contains("manifest sidecar"),
        "unexpected replacement-sidecar retry error: {retry_error}"
    );
    assert_eq!(
        fs::read(&sidecar).map_err(|error| error.to_string())?,
        sentinel
    );
    assert_eq!(
        fs::read(&relocated_sidecar).map_err(|error| error.to_string())?,
        exact_sidecar_before
    );
    assert_eq!(
        git_line(
            &fixture.source_root,
            &["show-ref", "--verify", "--hash", &proof_ref],
        )?,
        authority.merge_proof.target_commit
    );
    assert_eq!(
        reopened.lookup_worktree_removal(&fixture.lookup)?,
        Some(removed),
        "replacement-sidecar retry changed the tombstone"
    );
    Ok(())
}

#[test]
pub(super) fn finalized_offline_common_dir_defers_then_converges_cleanup() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let mut fault = FailOnceAt {
        target: unix::FaultBoundary::Finalized,
        tripped: false,
    };
    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut fault,
    )
    .expect_err("finalized boundary fault should interrupt cleanup");
    assert!(error.contains("injected crash after Finalized"));
    assert!(fault.tripped);

    let removed = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "finalized fault lost the removal tombstone".to_string())?;
    if !matches!(removed, CodeWorktreeRemovalRecord::Removed(_)) {
        return Err("finalized fault did not preserve a Removed tombstone".to_string());
    }
    let authority = removed.authority();
    let common_dir = Path::new(&authority.physical.git_admin_parent)
        .parent()
        .ok_or_else(|| "Git-admin parent has no common directory".to_string())?
        .to_path_buf();
    let offline_common_dir = fixture._directory.path().join("offline-common-dir");
    let sidecar = fixture
        .app_data
        .join("code")
        .join("removal-manifests-v1")
        .join(format!("{}.json", authority.physical_manifest_digest));
    let sidecar_before = fs::read(&sidecar).map_err(|error| error.to_string())?;
    let proof_ref_relative = Path::new("refs")
        .join("schoolx")
        .join("removal-claims")
        .join(&authority.removal_id);
    fs::rename(&common_dir, &offline_common_dir)
        .map_err(|error| format!("failed to take common directory offline: {error}"))?;
    assert!(offline_common_dir.join(&proof_ref_relative).is_file());

    let reopened = CodeThreadBindingStore::for_app_data(&fixture.app_data)?;
    unix::recover_all(&reopened, &fixture.nest_root, &mut unix::NoopFaultHook)?;
    assert_eq!(
        fs::read(&sidecar).map_err(|error| error.to_string())?,
        sidecar_before,
        "offline cleanup changed its durable sidecar"
    );
    assert!(
        offline_common_dir.join(&proof_ref_relative).is_file(),
        "offline cleanup changed the inaccessible proof ref"
    );
    assert_eq!(
        reopened.lookup_worktree_removal(&fixture.lookup)?,
        Some(removed.clone())
    );

    fs::rename(&offline_common_dir, &common_dir)
        .map_err(|error| format!("failed to restore common directory: {error}"))?;
    unix::recover_all(&reopened, &fixture.nest_root, &mut unix::NoopFaultHook)?;
    assert_removed(&fixture, &removed)
}

#[test]
pub(super) fn finalized_cleanup_preserves_replacement_proof_ref() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let mut fault = FailOnceAt {
        target: unix::FaultBoundary::Finalized,
        tripped: false,
    };
    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut fault,
    )
    .expect_err("finalized boundary fault should interrupt proof-ref cleanup");
    assert!(error.contains("injected crash after Finalized"));
    assert!(fault.tripped);

    let removed = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "finalized fault lost the removal tombstone".to_string())?;
    if !matches!(removed, CodeWorktreeRemovalRecord::Removed(_)) {
        return Err("finalized fault did not preserve a removed tombstone".to_string());
    }
    let authority = removed.authority();
    let proof_ref = format!("refs/schoolx/removal-claims/{}", authority.removal_id);
    let replacement = git_line(&fixture.source_root, &["rev-parse", "HEAD^{tree}"])?;
    if replacement == authority.merge_proof.target_commit {
        return Err("replacement proof-ref object unexpectedly equals target commit".to_string());
    }
    test_git(
        &fixture.source_root,
        &[
            "update-ref",
            &proof_ref,
            &replacement,
            &authority.merge_proof.target_commit,
        ],
    )?;
    let sidecar = fixture
        .app_data
        .join("code")
        .join("removal-manifests-v1")
        .join(format!("{}.json", authority.physical_manifest_digest));
    assert!(sidecar.is_file(), "finalized cleanup sidecar was missing");

    let reopened = CodeThreadBindingStore::for_app_data(&fixture.app_data)?;
    let recovery_error = unix::recover_all(&reopened, &fixture.nest_root, &mut unix::NoopFaultHook)
        .expect_err("cleanup must reject a different-OID proof-ref replacement");
    assert!(
        recovery_error.contains("proof ref replacement was preserved during cleanup"),
        "unexpected cleanup rejection: {recovery_error}"
    );
    assert_eq!(
        git_line(
            &fixture.source_root,
            &["show-ref", "--verify", "--hash", &proof_ref],
        )?,
        replacement,
        "cleanup changed a replacement proof ref"
    );
    assert!(sidecar.is_file(), "cleanup removed its retry sidecar");
    assert_eq!(
        reopened.lookup_worktree_removal(&fixture.lookup)?,
        Some(removed),
        "cleanup changed the finalized removal tombstone"
    );
    Ok(())
}

#[test]
pub(super) fn finalized_cleanup_rejects_and_preserves_symbolic_proof_ref() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let mut fault = FailOnceAt {
        target: unix::FaultBoundary::Finalized,
        tripped: false,
    };
    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut fault,
    )
    .expect_err("finalized boundary fault should interrupt proof-ref cleanup");
    assert!(error.contains("injected crash after Finalized"));
    assert!(fault.tripped);

    let removed = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "finalized fault lost the removal tombstone".to_string())?;
    if !matches!(removed, CodeWorktreeRemovalRecord::Removed(_)) {
        return Err("finalized fault did not preserve a Removed tombstone".to_string());
    }
    let authority = removed.authority();
    let proof_ref = format!("refs/schoolx/removal-claims/{}", authority.removal_id);
    let target_ref = authority.merge_proof.target_ref.clone();
    let sidecar = fixture
        .app_data
        .join("code")
        .join("removal-manifests-v1")
        .join(format!("{}.json", authority.physical_manifest_digest));
    let sidecar_before = fs::read(&sidecar).map_err(|error| error.to_string())?;
    test_git(
        &fixture.source_root,
        &["symbolic-ref", &proof_ref, &target_ref],
    )?;
    assert_eq!(
        git_line(&fixture.source_root, &["symbolic-ref", &proof_ref])?,
        target_ref
    );
    assert_eq!(
        git_line(
            &fixture.source_root,
            &["show-ref", "--verify", "--hash", &proof_ref],
        )?,
        authority.merge_proof.target_commit,
        "symbolic replacement did not resolve to the authorized target OID"
    );

    let reopened = CodeThreadBindingStore::for_app_data(&fixture.app_data)?;
    let recovery_error = unix::recover_all(&reopened, &fixture.nest_root, &mut unix::NoopFaultHook)
        .expect_err("cleanup must reject symbolic proof-ref authority");
    assert!(
        recovery_error.contains("proof ref is symbolic or has ambiguous raw authority"),
        "unexpected symbolic-ref cleanup rejection: {recovery_error}"
    );
    assert_eq!(
        git_line(&fixture.source_root, &["symbolic-ref", &proof_ref])?,
        target_ref,
        "cleanup changed the symbolic proof ref"
    );
    assert_eq!(
        fs::read(&sidecar).map_err(|error| error.to_string())?,
        sidecar_before,
        "cleanup changed the sidecar after symbolic-ref rejection"
    );
    assert_eq!(
        reopened.lookup_worktree_removal(&fixture.lookup)?,
        Some(removed),
        "symbolic-ref cleanup changed the removal tombstone"
    );
    Ok(())
}

#[test]
pub(super) fn finalized_cleanup_preserves_loose_proof_ref_symlink_replacement() -> Result<(), String>
{
    let fixture = prepare_fixture()?;
    let sibling_before = snapshot_tree(&fixture.sibling_root)?;
    let transcript_before = fs::read(&fixture.transcript).map_err(|error| error.to_string())?;
    let mut fault = FailOnceAt {
        target: unix::FaultBoundary::Finalized,
        tripped: false,
    };
    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut fault,
    )
    .expect_err("finalized boundary fault should interrupt proof-ref cleanup");
    assert!(error.contains("injected crash after Finalized"));
    assert!(fault.tripped);

    let removed = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "finalized fault lost the removal tombstone".to_string())?;
    if !matches!(removed, CodeWorktreeRemovalRecord::Removed(_)) {
        return Err("finalized fault did not preserve a removed tombstone".to_string());
    }
    let authority = removed.authority();
    let loose_ref = fixture
        .source_root
        .join(".git")
        .join("refs")
        .join("schoolx")
        .join("removal-claims")
        .join(&authority.removal_id);
    let loose_before = fs::read(&loose_ref)
        .map_err(|error| format!("finalized proof ref was not a loose file: {error}"))?;
    let expected_bytes = format!("{}\n", authority.merge_proof.target_commit).into_bytes();
    assert_eq!(loose_before, expected_bytes);
    let replacement_target = fixture._directory.path().join("replacement-proof-ref.txt");
    fs::write(&replacement_target, &expected_bytes).map_err(|error| error.to_string())?;
    fs::remove_file(&loose_ref).map_err(|error| error.to_string())?;
    std::os::unix::fs::symlink(&replacement_target, &loose_ref)
        .map_err(|error| format!("failed to install loose proof-ref symlink: {error}"))?;
    let sidecar = fixture
        .app_data
        .join("code")
        .join("removal-manifests-v1")
        .join(format!("{}.json", authority.physical_manifest_digest));
    assert!(sidecar.is_file(), "finalized cleanup sidecar was missing");

    let reopened = CodeThreadBindingStore::for_app_data(&fixture.app_data)?;
    let recovery_error = unix::recover_all(&reopened, &fixture.nest_root, &mut unix::NoopFaultHook)
        .expect_err("cleanup must reject a filesystem-symlink proof-ref replacement");
    assert!(
        recovery_error.contains("failed to pin manifest file"),
        "unexpected cleanup rejection: {recovery_error}"
    );
    assert!(
        fs::symlink_metadata(&loose_ref)
            .map_err(|error| error.to_string())?
            .file_type()
            .is_symlink(),
        "cleanup replaced or removed the loose proof-ref symlink"
    );
    assert_eq!(
        fs::read_link(&loose_ref).map_err(|error| error.to_string())?,
        replacement_target
    );
    assert_eq!(
        fs::read(&replacement_target).map_err(|error| error.to_string())?,
        expected_bytes,
        "cleanup changed the symlink target bytes"
    );
    assert!(sidecar.is_file(), "cleanup removed its retry sidecar");
    assert_eq!(
        reopened.lookup_worktree_removal(&fixture.lookup)?,
        Some(removed),
        "cleanup changed the finalized removal tombstone"
    );
    assert_eq!(snapshot_tree(&fixture.sibling_root)?, sibling_before);
    assert_eq!(
        fs::read(&fixture.transcript).map_err(|error| error.to_string())?,
        transcript_before
    );
    Ok(())
}

#[test]
pub(super) fn assume_unchanged_content_drift_is_not_deletion_authority() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    test_git(
        &fixture.managed_root,
        &["update-index", "--assume-unchanged", "--", "README.md"],
    )?;
    fs::write(
        fixture.managed_root.join("README.md"),
        b"locally valuable bytes hidden from status\n",
    )
    .map_err(|error| error.to_string())?;
    assert!(test_git(&fixture.managed_root, &["status", "--porcelain=v1", "-z"])?.is_empty());
    let root_before = snapshot_tree(&fixture.managed_root)?;
    let admin_before = snapshot_tree(
        linked_admin_entry(&fixture.managed_root)?
            .parent()
            .ok_or_else(|| "Git-admin entry has no parent".to_string())?,
    )?;

    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut unix::NoopFaultHook,
    )
    .expect_err("assume-unchanged content drift must reject removal");

    assert!(
        error.contains("does not match the exact Git object"),
        "unexpected rejection: {error}"
    );
    assert!(fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .is_none());
    assert_eq!(snapshot_tree(&fixture.managed_root)?, root_before);
    assert_eq!(
        snapshot_tree(
            linked_admin_entry(&fixture.managed_root)?
                .parent()
                .ok_or_else(|| "Git-admin entry has no parent".to_string())?,
        )?,
        admin_before
    );
    assert!(!fixture
        .app_data
        .join("code")
        .join("removal-manifests-v1")
        .exists());
    Ok(())
}

#[test]
pub(super) fn ignored_entry_rejects_claim_with_zero_git_store_or_filesystem_mutation(
) -> Result<(), String> {
    let fixture = prepare_fixture()?;
    fs::write(
        fixture.managed_root.join("ignored-output"),
        b"must never be inferred as disposable\n",
    )
    .map_err(|error| error.to_string())?;
    assert!(
        test_git(&fixture.managed_root, &["status", "--porcelain=v1", "-z"],)?.is_empty(),
        "fixture ignored entry unexpectedly appeared in ordinary status"
    );

    let root_parent = fixture
        .managed_root
        .parent()
        .ok_or_else(|| "managed root has no parent".to_string())?;
    let admin_entry = linked_admin_entry(&fixture.managed_root)?;
    let admin_parent = admin_entry
        .parent()
        .ok_or_else(|| "Git-admin entry has no parent".to_string())?;
    let store_before = fs::read(fixture.store.store_path()).map_err(|error| error.to_string())?;
    let store_modified_before = fs::metadata(fixture.store.store_path())
        .and_then(|metadata| metadata.modified())
        .map_err(|error| error.to_string())?;
    let root_parent_before = snapshot_tree(root_parent)?;
    let admin_parent_before = snapshot_tree(admin_parent)?;
    let sibling_before = snapshot_tree(&fixture.sibling_root)?;
    let transcript_before = fs::read(&fixture.transcript).map_err(|error| error.to_string())?;
    let refs_before = test_git(
        &fixture.source_root,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
    )?;

    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut unix::NoopFaultHook,
    )
    .expect_err("ignored entry must reject physical-removal claim");
    assert!(
        error.contains("rejects unexpected worktree entry"),
        "unexpected rejection: {error}"
    );

    assert!(fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .is_none());
    assert_eq!(
        fs::read(fixture.store.store_path()).map_err(|error| error.to_string())?,
        store_before,
        "binding store bytes changed on rejected claim"
    );
    assert_eq!(
        fs::metadata(fixture.store.store_path())
            .and_then(|metadata| metadata.modified())
            .map_err(|error| error.to_string())?,
        store_modified_before,
        "binding store mtime changed on rejected claim"
    );
    assert_eq!(snapshot_tree(root_parent)?, root_parent_before);
    assert_eq!(snapshot_tree(admin_parent)?, admin_parent_before);
    assert_eq!(snapshot_tree(&fixture.sibling_root)?, sibling_before);
    assert_eq!(
        fs::read(&fixture.transcript).map_err(|error| error.to_string())?,
        transcript_before
    );
    assert_eq!(
        test_git(
            &fixture.source_root,
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        )?,
        refs_before,
        "Git refs changed on rejected claim"
    );
    assert!(
        !fixture
            .app_data
            .join("code")
            .join("removal-manifests-v1")
            .exists(),
        "rejected claim persisted a manifest sidecar"
    );
    Ok(())
}
