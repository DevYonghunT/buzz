//! Read-only, exact-scope inventory of native-managed SchoolX Code worktrees.

use std::path::Path;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::bindings::{
    CodeExecutionMode, CodeThreadBinding, CodeThreadBindingScope, CodeThreadBindingStore,
    CodeThreadLifecycleStatus, CodeThreadPreparation, CodeThreadPreparationOperation,
    CodeThreadPreparationState,
};
use super::worktrees::{
    prove_binding_merge_target_before, revalidate_execution_root_before, CodeMergeProofOutcome,
    CodeWorktreeDescriptor, CodeWorktreeStatus,
};

const INVENTORY_INSPECTION_BUDGET: Duration = Duration::from_secs(30);
const MAX_INVENTORY_ERROR_BYTES: usize = 512;
const TRUNCATED_ERROR_SUFFIX: &str = " [truncated]";

/// Exact public coordinate accepted by the managed-worktree inventory.
///
/// Paths, descriptors, lifecycle claims, and removal assertions remain native
/// authority and cannot be supplied by the webview.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeWorktreesListInput {
    /// Community, project, and repository scope read from binding store v4.
    pub scope: CodeThreadBindingScope,
}

/// Durable native record that owns one inventory row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CodeWorktreeInventoryAuthority {
    /// A committed Codex thread binding and its persisted five-state lifecycle.
    Binding {
        /// Exact Codex thread identifier stored with the managed root.
        thread_id: String,
        /// Public projection of the native lifecycle journal.
        lifecycle: CodeThreadLifecycleStatus,
    },
    /// An unfinished native start or fork reservation.
    Preparation {
        /// Exact durable preparation UUID.
        preparation_id: String,
        /// Native operation that reserved the managed destination.
        operation: CodeThreadPreparationOperation,
        /// Whether the operation is prepared or requires sticky recovery.
        state: CodeThreadPreparationState,
        /// Exact fork source, absent for root-thread start.
        source_thread_id: Option<String>,
    },
}

/// Result of inspecting one native-derived managed descriptor.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CodeWorktreeInspection {
    /// The root remained inside the active nest and passed all Git checks.
    Available {
        /// Current immutable commit checked out at `HEAD`.
        head_commit: String,
        /// Current branch, or `None` when `HEAD` is detached.
        branch: Option<String>,
        /// Whether tracked or untracked files currently differ from `HEAD`.
        dirty: bool,
    },
    /// This row failed validation without suppressing its healthy peers.
    Unavailable {
        /// Native validation detail suitable for a read-only recovery surface.
        error: String,
    },
}

/// Closed native reasons that prevent managed-worktree removal.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodeWorktreeInventoryBlocker {
    /// A stable active binding still owns and can execute in this root.
    ActiveBinding,
    /// The binding lifecycle is transitional or unknown.
    LifecycleUnsettled,
    /// A start or fork preparation still reserves this root.
    UnfinishedPreparation,
    /// Defensive classification for a local checkout, which public inventory excludes.
    LocalCheckout,
    /// The persisted managed root could not be safely inspected.
    UnavailableRoot,
    /// The root contains tracked or untracked working-tree changes.
    DirtyRoot,
    /// `HEAD` is attached to a branch rather than detached.
    BranchAttached,
    /// Current `HEAD` differs from the persisted immutable base commit.
    HeadDrift,
    /// Native merge authority was absent, negative, unavailable, or unstable.
    MergeProofUnavailable,
}

/// One preserved managed root and its native-derived safe-removal eligibility.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeWorktreeInventoryRow {
    /// Exact durable scope that owns this row.
    pub scope: CodeThreadBindingScope,
    /// Binding or unfinished preparation that owns the managed root.
    pub authority: CodeWorktreeInventoryAuthority,
    /// Native-derived descriptor; callers never supply it to this list command.
    pub descriptor: CodeWorktreeDescriptor,
    /// Row-local filesystem and Git inspection result.
    pub inspection: CodeWorktreeInspection,
    /// Retention remains true even when explicit safe removal is eligible.
    pub preserved: bool,
    /// Whether an explicit remove call can presently pass all static gates.
    pub can_remove: bool,
    /// Complete deterministic set of native-derived removal blockers.
    pub blockers: Vec<CodeWorktreeInventoryBlocker>,
}

/// Project exact-scope managed bindings and unfinished managed preparations
/// into independent read-only rows.
pub fn list_worktree_inventory(
    store: &CodeThreadBindingStore,
    nest_root: &Path,
    scope: &CodeThreadBindingScope,
) -> Result<Vec<CodeWorktreeInventoryRow>, String> {
    let (bindings, preparations) = store.list_managed_inventory_authority(scope)?;
    let mut rows = Vec::with_capacity(bindings.len() + preparations.len());
    let deadline = Instant::now() + INVENTORY_INSPECTION_BUDGET;
    for snapshot in bindings {
        let descriptor = binding_descriptor(&snapshot.binding);
        let inspection = inspect_before_deadline(&descriptor, nest_root, deadline);
        let merge_proof = inventory_merge_proof(store, &snapshot, &inspection, nest_root, deadline);
        rows.push(project_binding_row(
            snapshot.binding,
            snapshot.status,
            inspection,
            merge_proof,
        ));
    }
    for preparation in preparations {
        let descriptor = preparation.descriptor();
        let inspection = inspect_before_deadline(&descriptor, nest_root, deadline);
        rows.push(project_preparation_row(preparation, inspection));
    }
    Ok(rows)
}

fn inspect_before_deadline(
    descriptor: &CodeWorktreeDescriptor,
    nest_root: &Path,
    deadline: Instant,
) -> Result<CodeWorktreeStatus, String> {
    if Instant::now() >= deadline {
        return Err("SchoolX Code worktree inspection budget was exhausted".to_string());
    }
    revalidate_execution_root_before(descriptor, nest_root, deadline)
}

fn binding_descriptor(binding: &CodeThreadBinding) -> CodeWorktreeDescriptor {
    CodeWorktreeDescriptor {
        execution_mode: binding.execution_mode,
        repository_identity: binding.repository_identity.clone(),
        execution_root: binding.execution_root.clone(),
        base_ref: binding.base_ref.clone(),
        worktree_id: binding.worktree_id.clone(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InventoryMergeProof {
    NotRequired,
    Proven,
    Unavailable,
}

fn inventory_merge_proof(
    store: &CodeThreadBindingStore,
    snapshot: &super::bindings::CodeThreadBindingLifecycle,
    inspection: &Result<CodeWorktreeStatus, String>,
    nest_root: &Path,
    deadline: Instant,
) -> InventoryMergeProof {
    if snapshot.status != CodeThreadLifecycleStatus::Archived {
        return InventoryMergeProof::NotRequired;
    }
    let Ok(status) = inspection else {
        return InventoryMergeProof::Unavailable;
    };
    let lookup = super::bindings::CodeThreadBindingLookupInput {
        scope: snapshot.binding.scope(),
        codex_thread_id: snapshot.binding.codex_thread_id.clone(),
    };
    if !matches!(store.lookup_worktree_removal(&lookup), Ok(None)) {
        return InventoryMergeProof::Unavailable;
    }
    let proof = match prove_binding_merge_target_before(store, &lookup, nest_root, deadline) {
        Ok(Some(CodeMergeProofOutcome::Proven(proof))) => proof,
        Ok(Some(CodeMergeProofOutcome::NotMerged)) | Ok(None) | Err(_) => {
            return InventoryMergeProof::Unavailable;
        }
    };
    if proof.repository_identity != snapshot.binding.repository_identity
        || snapshot.binding.worktree_id.as_deref() != Some(proof.worktree_id.as_str())
        || proof.head_commit != status.head_commit
    {
        return InventoryMergeProof::Unavailable;
    }

    let stable_lifecycle = matches!(
        store.lookup_with_lifecycle(&lookup),
        Ok(Some(current)) if current == *snapshot
    );
    let stable_authority = matches!(
        store.binding_merge_authority(&lookup),
        Ok(Some((binding, Some(target_ref))))
            if binding == snapshot.binding && target_ref == proof.target_ref
    );
    let no_removal = matches!(store.lookup_worktree_removal(&lookup), Ok(None));
    if stable_lifecycle && stable_authority && no_removal {
        InventoryMergeProof::Proven
    } else {
        InventoryMergeProof::Unavailable
    }
}

fn project_binding_row(
    binding: CodeThreadBinding,
    lifecycle: CodeThreadLifecycleStatus,
    inspection: Result<CodeWorktreeStatus, String>,
    merge_proof: InventoryMergeProof,
) -> CodeWorktreeInventoryRow {
    let scope = binding.scope();
    let descriptor = binding_descriptor(&binding);
    let authority = CodeWorktreeInventoryAuthority::Binding {
        thread_id: binding.codex_thread_id,
        lifecycle,
    };
    let primary = match lifecycle {
        CodeThreadLifecycleStatus::Active => Some(CodeWorktreeInventoryBlocker::ActiveBinding),
        CodeThreadLifecycleStatus::Archiving
        | CodeThreadLifecycleStatus::Unarchiving
        | CodeThreadLifecycleStatus::Unknown => {
            Some(CodeWorktreeInventoryBlocker::LifecycleUnsettled)
        }
        CodeThreadLifecycleStatus::Archived => None,
    };
    project_row(
        scope,
        authority,
        descriptor,
        primary,
        merge_proof,
        inspection,
    )
}

fn project_preparation_row(
    preparation: CodeThreadPreparation,
    inspection: Result<CodeWorktreeStatus, String>,
) -> CodeWorktreeInventoryRow {
    let scope = preparation.scope();
    let descriptor = preparation.descriptor();
    let authority = CodeWorktreeInventoryAuthority::Preparation {
        preparation_id: preparation.preparation_id,
        operation: preparation.operation,
        state: preparation.state,
        source_thread_id: preparation.source_thread_id,
    };
    project_row(
        scope,
        authority,
        descriptor,
        Some(CodeWorktreeInventoryBlocker::UnfinishedPreparation),
        InventoryMergeProof::NotRequired,
        inspection,
    )
}

fn project_row(
    scope: CodeThreadBindingScope,
    authority: CodeWorktreeInventoryAuthority,
    descriptor: CodeWorktreeDescriptor,
    primary_blocker: Option<CodeWorktreeInventoryBlocker>,
    merge_proof: InventoryMergeProof,
    inspection: Result<CodeWorktreeStatus, String>,
) -> CodeWorktreeInventoryRow {
    let mut blockers = Vec::with_capacity(5);
    if let Some(blocker) = primary_blocker {
        blockers.push(blocker);
    }
    if descriptor.execution_mode == CodeExecutionMode::Local {
        blockers.push(CodeWorktreeInventoryBlocker::LocalCheckout);
    }

    let inspection = match inspection {
        Ok(status) if status.descriptor == descriptor => {
            if status.dirty {
                blockers.push(CodeWorktreeInventoryBlocker::DirtyRoot);
            }
            if status.branch.is_some() {
                blockers.push(CodeWorktreeInventoryBlocker::BranchAttached);
            }
            if status.head_commit != descriptor.base_ref {
                blockers.push(CodeWorktreeInventoryBlocker::HeadDrift);
            }
            CodeWorktreeInspection::Available {
                head_commit: status.head_commit,
                branch: status.branch,
                dirty: status.dirty,
            }
        }
        Ok(_) => {
            blockers.push(CodeWorktreeInventoryBlocker::UnavailableRoot);
            CodeWorktreeInspection::Unavailable {
                error: "SchoolX Code worktree inspection returned a different descriptor"
                    .to_string(),
            }
        }
        Err(error) => {
            blockers.push(CodeWorktreeInventoryBlocker::UnavailableRoot);
            CodeWorktreeInspection::Unavailable {
                error: bounded_inventory_error(error),
            }
        }
    };
    match merge_proof {
        InventoryMergeProof::NotRequired => {}
        InventoryMergeProof::Proven => {
            blockers.retain(|blocker| *blocker != CodeWorktreeInventoryBlocker::HeadDrift);
        }
        InventoryMergeProof::Unavailable => {
            blockers.push(CodeWorktreeInventoryBlocker::MergeProofUnavailable);
        }
    }
    let can_remove = blockers.is_empty();

    CodeWorktreeInventoryRow {
        scope,
        authority,
        descriptor,
        inspection,
        preserved: true,
        can_remove,
        blockers,
    }
}

fn bounded_inventory_error(mut error: String) -> String {
    if error.len() <= MAX_INVENTORY_ERROR_BYTES {
        return error;
    }
    let mut end = MAX_INVENTORY_ERROR_BYTES.saturating_sub(TRUNCATED_ERROR_SUFFIX.len());
    while !error.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    error.truncate(end);
    error.push_str(TRUNCATED_ERROR_SUFFIX);
    error
}

#[cfg(test)]
mod tests;
