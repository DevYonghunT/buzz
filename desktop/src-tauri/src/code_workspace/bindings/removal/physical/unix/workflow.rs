use super::{
    delete::*, file_identity::*, manifest_capture::*, manifest_store::*, proof_refs::*, *,
};

pub(super) fn claimed_has_zero_mutation(
    record: &CodeWorktreeRemovalRecord,
    stored: &StoredManifest,
    nest_root: &Path,
    deadline: Instant,
) -> Result<bool, String> {
    let authority = record.authority();
    let layout = match pin_layout(&authority.binding, nest_root) {
        Ok(layout) => layout,
        Err(_) => return Ok(false),
    };
    if verify_layout_against_authority(&layout, authority, stored).is_err() {
        return Ok(false);
    }
    let original = named_directory_state(
        &layout.root_parent,
        worktree_name(authority)?,
        &stored.manifest.root_identity,
    )?;
    let quarantine = named_directory_state(
        &layout.root_parent,
        OsStr::new(&authority.physical.quarantine_name),
        &stored.manifest.root_identity,
    )?;
    let admin = named_directory_state(
        &layout.admin_parent,
        OsStr::new(&authority.physical.git_admin_entry),
        &stored.manifest.admin_identity,
    )?;
    let proof_ref = read_proof_ref(
        &layout.git_launch,
        &layout.common_dir,
        &layout.common_dir_path,
        authority,
        deadline,
    )?;
    Ok((original, quarantine, admin)
        == (
            CoordinateState::Expected,
            CoordinateState::Absent,
            CoordinateState::Expected,
        )
        && proof_ref.is_none())
}

pub(super) fn resume_record(
    store: &CodeThreadBindingStore,
    record: CodeWorktreeRemovalRecord,
    nest_root: &Path,
    deadline: Instant,
    hook: &mut dyn FaultHook,
) -> Result<CodeWorktreeRemovalRecord, String> {
    match record {
        CodeWorktreeRemovalRecord::Removed(_) => {
            cleanup_removed(store, &record, hook)?;
            Ok(record)
        }
        CodeWorktreeRemovalRecord::Claimed(_) => {
            let stored = load_manifest_sidecar(store, record.authority())?;
            verify_claimed_zero_mutation(&record, &stored, nest_root, deadline)?;
            let removing = store.mark_worktree_removal_removing(&record)?;
            hook.after(FaultBoundary::Removing)?;
            execute_removing(store, removing, &stored, nest_root, deadline, hook)
        }
        CodeWorktreeRemovalRecord::Removing(_) => {
            let stored = load_manifest_sidecar(store, record.authority())?;
            execute_removing(store, record, &stored, nest_root, deadline, hook)
        }
    }
}

pub(super) fn exact_merge_proof(
    store: &CodeThreadBindingStore,
    lookup: &CodeThreadBindingLookupInput,
    nest_root: &Path,
    deadline: Instant,
) -> Result<CodeMergeProofReceipt, String> {
    match prove_binding_merge_target_before(store, lookup, nest_root, deadline)? {
        Some(CodeMergeProofOutcome::Proven(proof)) => Ok(proof),
        Some(CodeMergeProofOutcome::NotMerged) => {
            Err("SchoolX Code worktree HEAD is not merged into its native target".to_string())
        }
        None => Err("SchoolX Code worktree has no native merge-target authority".to_string()),
    }
}

pub(super) fn claim_input(
    lookup: &CodeThreadBindingLookupInput,
    proof: CodeMergeProofReceipt,
    stored: &StoredManifest,
) -> CodeWorktreeRemovalClaimInput {
    CodeWorktreeRemovalClaimInput {
        lookup: lookup.clone(),
        merge_proof: proof,
        physical_manifest_digest: stored.digest.clone(),
        git_admin_parent: stored.manifest.git_admin_parent.clone(),
        git_admin_entry: stored.manifest.git_admin_entry.clone(),
    }
}

pub(super) fn verify_claimed_zero_mutation(
    record: &CodeWorktreeRemovalRecord,
    stored: &StoredManifest,
    nest_root: &Path,
    deadline: Instant,
) -> Result<(), String> {
    let authority = record.authority();
    let layout = pin_layout(&authority.binding, nest_root)?;
    verify_layout_against_authority(&layout, authority, stored)?;
    let original = named_directory_state(
        &layout.root_parent,
        worktree_name(authority)?,
        &stored.manifest.root_identity,
    )?;
    let quarantine = named_directory_state(
        &layout.root_parent,
        OsStr::new(&authority.physical.quarantine_name),
        &stored.manifest.root_identity,
    )?;
    let admin = named_directory_state(
        &layout.admin_parent,
        OsStr::new(&authority.physical.git_admin_entry),
        &stored.manifest.admin_identity,
    )?;
    if (original, quarantine, admin)
        != (
            CoordinateState::Expected,
            CoordinateState::Absent,
            CoordinateState::Expected,
        )
    {
        return Err(
            "SchoolX Code claimed removal no longer has a definitely-not-started physical state"
                .to_string(),
        );
    }
    if read_proof_ref(
        &layout.git_launch,
        &layout.common_dir,
        &layout.common_dir_path,
        authority,
        deadline,
    )?
    .is_some()
    {
        return Err(
            "SchoolX Code claimed removal unexpectedly has a physical proof ref".to_string(),
        );
    }
    let lookup = record.lookup();
    let current_proof = exact_merge_proof_for_record(&lookup, authority, nest_root, deadline)?;
    if current_proof != authority.merge_proof {
        return Err("SchoolX Code removal proof changed after claim".to_string());
    }
    let current = capture_manifest_for_binding(
        &authority.binding,
        &authority.merge_proof,
        nest_root,
        deadline,
    )?;
    if current != *stored {
        return Err("SchoolX Code removal manifest changed after claim".to_string());
    }
    Ok(())
}

pub(super) fn exact_merge_proof_for_record(
    _lookup: &CodeThreadBindingLookupInput,
    authority: &super::super::super::CodeWorktreeRemovalAuthority,
    nest_root: &Path,
    deadline: Instant,
) -> Result<CodeMergeProofReceipt, String> {
    let descriptor = crate::code_workspace::worktrees::CodeWorktreeDescriptor {
        execution_mode: authority.binding.execution_mode,
        repository_identity: authority.binding.repository_identity.clone(),
        execution_root: authority.binding.execution_root.clone(),
        base_ref: authority.binding.base_ref.clone(),
        worktree_id: authority.binding.worktree_id.clone(),
    };
    match crate::code_workspace::worktrees::prove_direct_local_ancestry_before(
        &descriptor,
        nest_root,
        &authority.merge_proof.target_ref,
        deadline,
    )? {
        CodeMergeProofOutcome::Proven(proof) => Ok(proof),
        CodeMergeProofOutcome::NotMerged => {
            Err("SchoolX Code removal ancestry proof is no longer valid".to_string())
        }
    }
}

pub(super) fn execute_removing(
    store: &CodeThreadBindingStore,
    removing: CodeWorktreeRemovalRecord,
    stored: &StoredManifest,
    nest_root: &Path,
    deadline: Instant,
    hook: &mut dyn FaultHook,
) -> Result<CodeWorktreeRemovalRecord, String> {
    let authority = removing.authority();
    let mut boundary = pin_recovery_boundary(authority, stored, nest_root)?;
    verify_recovery_boundary_paths(&boundary, authority, stored)?;
    let states = observe_coordinates(&boundary, authority, stored)?;

    match states {
        (CoordinateState::Expected, CoordinateState::Absent, CoordinateState::Expected) => {
            let lookup = removing.lookup();
            let proof = exact_merge_proof_for_record(&lookup, authority, nest_root, deadline)?;
            if proof != authority.merge_proof {
                return Err("SchoolX Code removing proof changed before first mutation".to_string());
            }
            let current = capture_manifest_for_binding(
                &authority.binding,
                &authority.merge_proof,
                nest_root,
                deadline,
            )?;
            if current != *stored {
                return Err(
                    "SchoolX Code removing manifest changed before first mutation".to_string(),
                );
            }
            verify_recovery_boundary_paths(&boundary, authority, stored)?;
            ensure_proof_ref(
                &boundary.git_launch,
                &boundary.common_dir,
                &boundary.common_dir_path,
                authority,
                deadline,
            )?;
            hook.after(FaultBoundary::ProofRefPinned)?;
            let proof = exact_merge_proof_for_record(&lookup, authority, nest_root, deadline)?;
            if proof != authority.merge_proof {
                return Err("SchoolX Code removal proof drifted after proof-ref pin".to_string());
            }
            let current = capture_manifest_for_binding(
                &authority.binding,
                &authority.merge_proof,
                nest_root,
                deadline,
            )?;
            if current != *stored {
                return Err("SchoolX Code removal manifest drifted after proof-ref pin".to_string());
            }
            verify_recovery_boundary_paths(&boundary, authority, stored)?;
            verify_named_directory(
                &boundary.root_parent,
                worktree_name(authority)?,
                &stored.manifest.root_identity,
            )?;
            quarantine_root(&boundary.root_parent, authority, stored)?;
            hook.after(FaultBoundary::Quarantined)?;
            boundary.root = Some(open_expected_directory_at(
                &boundary.root_parent,
                OsStr::new(&authority.physical.quarantine_name),
                &stored.manifest.root_identity,
                "removal quarantine",
            )?);
        }
        (CoordinateState::Absent, CoordinateState::Expected, CoordinateState::Expected) => {
            require_exact_proof_ref(
                &boundary.git_launch,
                &boundary.common_dir,
                &boundary.common_dir_path,
                authority,
                deadline,
            )?;
            boundary.root = Some(open_expected_directory_at(
                &boundary.root_parent,
                OsStr::new(&authority.physical.quarantine_name),
                &stored.manifest.root_identity,
                "removal quarantine",
            )?);
        }
        (CoordinateState::Absent, CoordinateState::Absent, CoordinateState::Expected)
        | (CoordinateState::Absent, CoordinateState::Absent, CoordinateState::Absent) => {
            require_exact_proof_ref(
                &boundary.git_launch,
                &boundary.common_dir,
                &boundary.common_dir_path,
                authority,
                deadline,
            )?;
        }
        _ => {
            return Err(
                "SchoolX Code removal coordinates contain a replacement or ambiguous state; recovery is sticky"
                    .to_string(),
            );
        }
    }

    let states = observe_coordinates(&boundary, authority, stored)?;
    if states.1 == CoordinateState::Expected {
        let mut path_guard = || verify_recovery_boundary_paths(&boundary, authority, stored);
        delete_manifest_tree(
            &boundary.root,
            &stored.manifest.root_entries,
            &stored.manifest.root_identity,
            hook,
            true,
            &mut path_guard,
        )?;
        verify_recovery_boundary_paths(&boundary, authority, stored)?;
        remove_named_root(
            &boundary.root_parent,
            OsStr::new(&authority.physical.quarantine_name),
            &stored.manifest.root_identity,
        )?;
        hook.after(FaultBoundary::RootDeleted)?;
    }

    verify_recovery_boundary_paths(&boundary, authority, stored)?;
    let states = observe_coordinates(&boundary, authority, stored)?;
    if states.0 != CoordinateState::Absent || states.1 != CoordinateState::Absent {
        return Err(
            "SchoolX Code original or quarantine coordinate was replaced during removal"
                .to_string(),
        );
    }
    if states.2 == CoordinateState::Expected {
        boundary.admin = Some(open_expected_directory_at(
            &boundary.admin_parent,
            OsStr::new(&authority.physical.git_admin_entry),
            &stored.manifest.admin_identity,
            "Git-admin entry",
        )?);
        let mut path_guard = || verify_recovery_boundary_paths(&boundary, authority, stored);
        delete_manifest_tree(
            &boundary.admin,
            &stored.manifest.admin_entries,
            &stored.manifest.admin_identity,
            hook,
            false,
            &mut path_guard,
        )?;
        verify_recovery_boundary_paths(&boundary, authority, stored)?;
        remove_named_root(
            &boundary.admin_parent,
            OsStr::new(&authority.physical.git_admin_entry),
            &stored.manifest.admin_identity,
        )?;
        hook.after(FaultBoundary::AdminDeleted)?;
    } else if states.2 == CoordinateState::Replacement {
        return Err("SchoolX Code Git-admin coordinate was replaced during removal".to_string());
    }

    verify_recovery_boundary_paths(&boundary, authority, stored)?;
    verify_final_absence_and_siblings(&boundary, authority, stored)?;
    hook.after(FaultBoundary::AbsenceVerified)?;
    let capability = VerifiedRemovalAbsence::new(removing.clone());
    let removed = store.finalize_worktree_removal_after_verified_absence(capability)?;
    hook.after(FaultBoundary::Finalized)?;
    cleanup_removed(store, &removed, hook)?;
    Ok(removed)
}

pub(super) fn cleanup_removed(
    store: &CodeThreadBindingStore,
    removed: &CodeWorktreeRemovalRecord,
    hook: &mut dyn FaultHook,
) -> Result<(), String> {
    let authority = removed.authority();
    let stored = match load_manifest_sidecar(store, authority) {
        Ok(stored) => stored,
        Err(error) if error.starts_with("absent:") => {
            return harden_manifest_absence(store, &authority.physical_manifest_digest)
        }
        Err(error) => return Err(error),
    };
    let common_dir = Path::new(&authority.physical.git_admin_parent)
        .parent()
        .ok_or_else(|| "SchoolX Code removal Git-admin parent has no common dir".to_string())?;
    match fs::symlink_metadata(common_dir) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // The physical removal is finalized, so runtime startup need not
            // be blocked by an offline/moved repository. Keep the sidecar as
            // the durable cleanup-pending marker and retry the exact ref
            // coordinate if the original repository returns.
            return Ok(());
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect removed tombstone common dir: {error}"
            ));
        }
    }
    if repository_identity(common_dir)? != authority.binding.repository_identity {
        return Err("SchoolX Code removed tombstone common-dir identity changed".to_string());
    }
    let common = open_directory_absolute(common_dir, "removal common dir")?;
    if !same_directory_identity(
        &directory_identity(&common)?,
        &stored.manifest.common_dir_identity,
    ) {
        return Err("SchoolX Code removed tombstone common dir was replaced".to_string());
    }
    let git_launch = RemovalGitLaunchAuthority::admit(&common)?;
    delete_proof_ref_if_matches(
        &git_launch,
        &common,
        common_dir,
        authority,
        Instant::now() + GIT_TIMEOUT,
    )?;
    hook.after(FaultBoundary::ProofRefCleaned)?;
    remove_manifest_sidecar(store, &stored)
}
