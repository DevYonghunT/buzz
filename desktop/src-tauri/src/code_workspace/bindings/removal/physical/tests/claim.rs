use super::*;

#[test]
pub(super) fn clean_linked_worktree_is_physically_removed_without_mutating_siblings_or_transcript(
) -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let source_head = git_line(&fixture.source_root, &["rev-parse", "HEAD"])?;
    let source_status = test_git(&fixture.source_root, &["status", "--porcelain=v1", "-z"])?;
    let sibling_before = snapshot_tree(&fixture.sibling_root)?;
    let transcript_before = fs::read(&fixture.transcript).map_err(|error| error.to_string())?;

    let removed = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut unix::NoopFaultHook,
    )?;

    assert_removed(&fixture, &removed)?;
    assert_eq!(
        snapshot_tree(&fixture.sibling_root)?,
        sibling_before,
        "sibling worktree bytes changed"
    );
    assert_eq!(
        fs::read(&fixture.transcript).map_err(|error| error.to_string())?,
        transcript_before,
        "Codex transcript bytes changed"
    );
    assert_eq!(
        git_line(&fixture.source_root, &["rev-parse", "HEAD"])?,
        source_head,
        "authorized branch changed"
    );
    assert_eq!(
        test_git(&fixture.source_root, &["status", "--porcelain=v1", "-z"])?,
        source_status,
        "source checkout changed"
    );
    Ok(())
}

#[test]
pub(super) fn tracked_external_symlink_removal_preserves_target_bytes() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let target_before = fs::read(&fixture.external_symlink_target)
        .map_err(|error| format!("failed to read external symlink target: {error}"))?;
    assert_eq!(
        fs::read_link(fixture.managed_root.join("README.link"))
            .map_err(|error| format!("failed to read tracked external symlink: {error}"))?,
        fixture.external_symlink_target
    );

    let removed = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut unix::NoopFaultHook,
    )?;

    assert_removed(&fixture, &removed)?;
    assert_eq!(
        fs::read(&fixture.external_symlink_target)
            .map_err(|error| format!("external symlink target was not preserved: {error}"))?,
        target_before
    );
    Ok(())
}

#[test]
pub(super) fn shared_clone_alternates_reject_claim_with_zero_mutation() -> Result<(), String> {
    let fixture = prepare_fixture_with_storage(TestRepositoryStorage::SharedClone)?;
    let alternates = fixture
        .source_root
        .join(".git")
        .join("objects")
        .join("info")
        .join("alternates");
    let alternate_bytes = fs::read(&alternates)
        .map_err(|error| format!("git clone --shared did not create alternates: {error}"))?;
    assert!(
        !alternate_bytes.is_empty(),
        "git clone --shared created an empty alternates file"
    );

    assert_claim_rejected_without_mutation(&fixture, "alternate object storage")?;
    assert_eq!(
        fs::read(&alternates).map_err(|error| error.to_string())?,
        alternate_bytes,
        "rejected claim changed the shared-clone alternates file"
    );
    Ok(())
}

#[test]
pub(super) fn git_worktree_lock_rejects_claim_with_zero_mutation() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let managed_root = path_string(&fixture.managed_root, "managed worktree")?;
    test_git(
        &fixture.source_root,
        &[
            "worktree",
            "lock",
            "--reason",
            "SchoolX adversarial removal test",
            &managed_root,
        ],
    )?;
    let admin = linked_admin_entry(&fixture.managed_root)?;
    assert!(
        admin.join("locked").is_file(),
        "git worktree lock did not create the Git-admin lock marker"
    );

    assert_claim_rejected_without_mutation(
        &fixture,
        "refuses a locked or concurrently mutated Git-admin entry",
    )
}

#[test]
pub(super) fn missing_local_head_blob_rejects_claim_with_zero_mutation() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let object_id = git_line(&fixture.source_root, &["rev-parse", "HEAD:README.md"])?;
    if object_id.len() < 3 || !object_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("fixture README blob object id was invalid".to_string());
    }
    let object_path = fixture
        .source_root
        .join(".git")
        .join("objects")
        .join(&object_id[..2])
        .join(&object_id[2..]);
    let object_bytes = fs::read(&object_path).map_err(|error| {
        format!("fixture README blob was not an ordinary loose object: {error}")
    })?;
    let parked_object = fixture._directory.path().join("parked-readme-blob");
    fs::rename(&object_path, &parked_object)
        .map_err(|error| format!("failed to hide fixture README blob: {error}"))?;
    assert!(
        fs::symlink_metadata(&object_path).is_err(),
        "fixture README blob remained locally available"
    );

    assert_claim_rejected_without_mutation(&fixture, "requires every HEAD blob to exist locally")?;
    assert!(
        fs::symlink_metadata(&object_path).is_err(),
        "rejected claim recreated the missing local blob"
    );
    assert_eq!(
        fs::read(&parked_object).map_err(|error| error.to_string())?,
        object_bytes,
        "rejected claim changed the parked blob bytes"
    );
    Ok(())
}

#[test]
pub(super) fn untracked_empty_directory_rejects_claim_with_zero_mutation() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    fs::create_dir(fixture.managed_root.join("untracked-empty-directory"))
        .map_err(|error| error.to_string())?;
    assert!(
        test_git(&fixture.managed_root, &["status", "--porcelain=v1", "-z"])?.is_empty(),
        "Git unexpectedly reported an empty untracked directory"
    );

    assert_claim_rejected_without_mutation(&fixture, "rejects unexpected worktree entry")
}

#[test]
pub(super) fn ignored_fifo_rejects_claim_with_zero_mutation() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let fifo_path = fixture.managed_root.join("ignored-output");
    let executable = crate::managed_agents::resolve_command("mkfifo")
        .ok_or_else(|| "mkfifo executable was not found".to_string())?;
    let output = Command::new(executable)
        .arg(&fifo_path)
        .output()
        .map_err(|error| format!("failed to create adversarial FIFO: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "failed to create adversarial FIFO: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    assert!(
        test_git(&fixture.managed_root, &["status", "--porcelain=v1", "-z"])?.is_empty(),
        "fixture ignored special entry unexpectedly appeared in ordinary status"
    );
    assert!(
        fs::symlink_metadata(&fifo_path)
            .map_err(|error| error.to_string())?
            .file_type()
            .is_fifo(),
        "adversarial entry was not a FIFO"
    );

    assert_claim_rejected_without_mutation(&fixture, "rejects unexpected worktree entry")
}

#[test]
pub(super) fn child_process_crash_reopen_matrix_recovers_every_durable_boundary(
) -> Result<(), String> {
    for boundary in CrashBoundary::ALL {
        let result = (|| -> Result<(), String> {
            let fixture = prepare_fixture()?;
            let sibling_before = snapshot_tree(&fixture.sibling_root)?;
            let transcript_before =
                fs::read(&fixture.transcript).map_err(|error| error.to_string())?;
            let source_head = git_line(&fixture.source_root, &["rev-parse", "HEAD"])?;
            let source_status =
                test_git(&fixture.source_root, &["status", "--porcelain=v1", "-z"])?;

            spawn_crash_child(&crash_request_for(&fixture, boundary)?)?;

            let interrupted_store = CodeThreadBindingStore::for_app_data(&fixture.app_data)?;
            let interrupted = interrupted_store
                .lookup_worktree_removal(&fixture.lookup)?
                .ok_or_else(|| "crash did not preserve a removal journal".to_string())?;
            match (&interrupted, boundary) {
                (CodeWorktreeRemovalRecord::Claimed(_), CrashBoundary::Claimed)
                | (
                    CodeWorktreeRemovalRecord::Removed(_),
                    CrashBoundary::Finalized | CrashBoundary::ProofRefCleaned,
                )
                | (
                    CodeWorktreeRemovalRecord::Removing(_),
                    CrashBoundary::Removing
                    | CrashBoundary::ProofRefPinned
                    | CrashBoundary::Quarantined
                    | CrashBoundary::RootEntryDeleted
                    | CrashBoundary::RootDeleted
                    | CrashBoundary::AdminEntryDeleted
                    | CrashBoundary::AdminDeleted
                    | CrashBoundary::AbsenceVerified,
                ) => {}
                _ => {
                    return Err(format!(
                        "crash at {boundary:?} persisted the wrong journal state: {interrupted:?}"
                    ))
                }
            }
            let removal_id = interrupted.authority().removal_id.clone();

            let reopened = CodeThreadBindingStore::for_app_data(&fixture.app_data)?;
            unix::recover_all(&reopened, &fixture.nest_root, &mut unix::NoopFaultHook)?;
            let removed = reopened
                .lookup_worktree_removal(&fixture.lookup)?
                .ok_or_else(|| "recovery lost the removal tombstone".to_string())?;
            if !matches!(removed, CodeWorktreeRemovalRecord::Removed(_)) {
                return Err("recovery did not finish with a removed tombstone".to_string());
            }
            if removed.authority().removal_id != removal_id {
                return Err("recovery replaced the durable removal id".to_string());
            }
            assert_removed(&fixture, &removed)?;
            assert_eq!(
                snapshot_tree(&fixture.sibling_root)?,
                sibling_before,
                "sibling worktree bytes changed after {boundary:?}"
            );
            assert_eq!(
                fs::read(&fixture.transcript).map_err(|error| error.to_string())?,
                transcript_before,
                "Codex transcript bytes changed after {boundary:?}"
            );
            assert_eq!(
                git_line(&fixture.source_root, &["rev-parse", "HEAD"])?,
                source_head,
                "authorized branch changed after {boundary:?}"
            );
            assert_eq!(
                test_git(&fixture.source_root, &["status", "--porcelain=v1", "-z"])?,
                source_status,
                "source checkout changed after {boundary:?}"
            );
            Ok(())
        })();
        result.map_err(|error| format!("crash/reopen matrix failed at {boundary:?}: {error}"))?;
    }
    Ok(())
}
