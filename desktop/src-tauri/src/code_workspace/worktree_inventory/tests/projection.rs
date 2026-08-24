use super::*;

#[test]
fn worktree_inventory_projects_closed_blockers_without_removal_authority() -> Result<(), String> {
    let descriptor = CodeWorktreeDescriptor {
        execution_mode: CodeExecutionMode::Worktree,
        repository_identity: "a".repeat(64),
        execution_root: "/native/managed".to_string(),
        base_ref: "b".repeat(40),
        worktree_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
    };
    let owner = CodeThreadBindingScope {
        community_id: "community-a".to_string(),
        project_dtag: "project-a".to_string(),
        repository_identity: descriptor.repository_identity.clone(),
    };
    let bound = binding(&owner, "thread-a", &descriptor);
    let clean = available_status(descriptor.clone(), &descriptor.base_ref, None, false);

    for lifecycle in [
        CodeThreadLifecycleStatus::Archiving,
        CodeThreadLifecycleStatus::Unarchiving,
        CodeThreadLifecycleStatus::Unknown,
    ] {
        let row = project_binding_row(
            bound.clone(),
            lifecycle,
            Ok(clean.clone()),
            InventoryMergeProof::NotRequired,
        );
        assert_eq!(
            row.blockers,
            vec![CodeWorktreeInventoryBlocker::LifecycleUnsettled]
        );
        assert!(row.preserved);
        assert!(!row.can_remove);
    }

    let active = project_binding_row(
        bound.clone(),
        CodeThreadLifecycleStatus::Active,
        Ok(clean.clone()),
        InventoryMergeProof::NotRequired,
    );
    assert_eq!(
        active.blockers,
        vec![CodeWorktreeInventoryBlocker::ActiveBinding]
    );

    let archived = project_binding_row(
        bound.clone(),
        CodeThreadLifecycleStatus::Archived,
        Ok(clean.clone()),
        InventoryMergeProof::Unavailable,
    );
    assert_eq!(
        archived.blockers,
        vec![CodeWorktreeInventoryBlocker::MergeProofUnavailable]
    );
    assert!(!archived.can_remove);

    let eligible = project_binding_row(
        bound.clone(),
        CodeThreadLifecycleStatus::Archived,
        Ok(clean.clone()),
        InventoryMergeProof::Proven,
    );
    assert!(eligible.blockers.is_empty());
    assert!(eligible.preserved);
    assert!(eligible.can_remove);

    let merged_head_drift = project_binding_row(
        bound.clone(),
        CodeThreadLifecycleStatus::Archived,
        Ok(available_status(
            descriptor.clone(),
            &"c".repeat(40),
            None,
            false,
        )),
        InventoryMergeProof::Proven,
    );
    assert!(merged_head_drift.blockers.is_empty());
    assert!(merged_head_drift.can_remove);

    let changed = project_binding_row(
        bound.clone(),
        CodeThreadLifecycleStatus::Archived,
        Ok(available_status(
            descriptor.clone(),
            &"c".repeat(40),
            Some("topic"),
            true,
        )),
        InventoryMergeProof::Unavailable,
    );
    assert_eq!(
        changed.blockers,
        vec![
            CodeWorktreeInventoryBlocker::DirtyRoot,
            CodeWorktreeInventoryBlocker::BranchAttached,
            CodeWorktreeInventoryBlocker::HeadDrift,
            CodeWorktreeInventoryBlocker::MergeProofUnavailable,
        ]
    );

    let unavailable = project_binding_row(
        bound.clone(),
        CodeThreadLifecycleStatus::Archived,
        Err("missing managed root".to_string()),
        InventoryMergeProof::Unavailable,
    );
    assert_eq!(
        unavailable.blockers,
        vec![
            CodeWorktreeInventoryBlocker::UnavailableRoot,
            CodeWorktreeInventoryBlocker::MergeProofUnavailable,
        ]
    );
    assert!(matches!(
        unavailable.inspection,
        CodeWorktreeInspection::Unavailable { .. }
    ));

    let oversized = project_binding_row(
        bound.clone(),
        CodeThreadLifecycleStatus::Archived,
        Err("가".repeat(MAX_INVENTORY_ERROR_BYTES)),
        InventoryMergeProof::Unavailable,
    );
    let CodeWorktreeInspection::Unavailable { error } = oversized.inspection else {
        return Err("oversized inspection error remained available".to_string());
    };
    assert!(error.len() <= MAX_INVENTORY_ERROR_BYTES);
    assert!(error.ends_with(TRUNCATED_ERROR_SUFFIX));

    let mut local = bound;
    local.execution_mode = CodeExecutionMode::Local;
    local.worktree_id = None;
    let local_status = available_status(
        CodeWorktreeDescriptor {
            execution_mode: CodeExecutionMode::Local,
            repository_identity: local.repository_identity.clone(),
            execution_root: local.execution_root.clone(),
            base_ref: local.base_ref.clone(),
            worktree_id: None,
        },
        &local.base_ref,
        Some("main"),
        false,
    );
    let local = project_binding_row(
        local,
        CodeThreadLifecycleStatus::Archived,
        Ok(local_status),
        InventoryMergeProof::Unavailable,
    );
    assert!(local
        .blockers
        .contains(&CodeWorktreeInventoryBlocker::LocalCheckout));
    Ok(())
}
