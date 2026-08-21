//! Restartable cleanup and live publish proofs for recovered transactions.

use std::fs;
use std::path::Path;

use super::super::super::journal::{GitJournalArtifactEvidence, GitJournalPhase, GitJournalRecord};
use super::super::super::owned_lock::{
    OwnedLockPlacement, OwnedPublishedArtifact, PinnedAdminDirectory, StandardLockKind,
};
use super::super::super::private_artifact::PrivateArtifactStore;
use super::super::super::protocol::CodeGitOperation;
use super::repository::require_owned_admin;
use super::{owned_artifact, owned_lock, private_identity};

/// Prove the crash-window state immediately after the exact target rename.
pub(super) fn prove_expected_publish(
    record: &GitJournalRecord,
    admin: &PinnedAdminDirectory,
) -> Result<(), String> {
    if record.phase != GitJournalPhase::LocksReady {
        return Err("Live publish proof requires the durable locks-ready phase".to_string());
    }
    let (target, target_kind, guard, guard_kind) = publish_evidence(record)?;
    match admin.classify_owned_lock(&owned_lock(target, target_kind))? {
        OwnedLockPlacement::Published(_) => {}
        OwnedLockPlacement::Locked(_) | OwnedLockPlacement::Released(_) => {
            return Err("Expected Git destination is not the exact published inode".to_string());
        }
    }
    match admin.classify_owned_lock(&owned_lock(guard, guard_kind))? {
        OwnedLockPlacement::Locked(_) => Ok(()),
        OwnedLockPlacement::Published(_) | OwnedLockPlacement::Released(_) => {
            Err("Git publish guard is not the exact owned lock".to_string())
        }
    }
}

/// Remove every exact-owned artifact before recording an uncertain outcome.
pub(in crate::code_workspace::git_write::transaction) fn cleanup_before_uncertain(
    app_data_dir: &Path,
    record: &GitJournalRecord,
) -> Result<(), String> {
    if matches!(
        record.phase,
        GitJournalPhase::IndexPublished | GitJournalPhase::HeadPublished
    ) {
        return cleanup_after_publish(app_data_dir, record);
    }
    collect_cleanup([
        cleanup_unpublished_admin(record),
        cleanup_private(app_data_dir, record),
    ])
}

/// Converge all exact-owned cleanup after a durable or live-proven publish.
pub(in crate::code_workspace::git_write::transaction) fn cleanup_after_publish(
    app_data_dir: &Path,
    record: &GitJournalRecord,
) -> Result<(), String> {
    let admin = pin_recorded_admin(record)?;
    let (target, target_kind, guard, guard_kind) = publish_evidence(record)?;

    // The guard is never a destination and is safe to remove first. Both
    // primitives accept an exact, already-absent retry after a partial crash.
    let guard_result = admin
        .cleanup_standard_lock(&owned_lock(guard, guard_kind))
        .map(|_| ());
    let target_result = admin
        .remove_published_artifact(&OwnedPublishedArtifact {
            artifact_component: target.name.clone(),
            kind: target_kind,
            identity: super::owned_identity(target),
        })
        .map(|_| ());
    collect_cleanup([
        guard_result,
        target_result,
        cleanup_private(app_data_dir, record),
    ])
}

fn cleanup_unpublished_admin(record: &GitJournalRecord) -> Result<(), String> {
    let (Some(index), Some(head)) = (
        record.artifacts.index_artifact.as_ref(),
        record.artifacts.head_artifact.as_ref(),
    ) else {
        if record.artifacts.index_artifact.is_some() || record.artifacts.head_artifact.is_some() {
            return Err("Recovery journal contains partial owned artifact evidence".to_string());
        }
        return Ok(());
    };
    let admin = pin_recorded_admin(record)?;
    collect_cleanup([
        cleanup_unpublished_one(&admin, head, StandardLockKind::Head),
        cleanup_unpublished_one(&admin, index, StandardLockKind::Index),
    ])
}

fn cleanup_unpublished_one(
    admin: &PinnedAdminDirectory,
    evidence: &GitJournalArtifactEvidence,
    kind: StandardLockKind,
) -> Result<(), String> {
    match evidence.link_count {
        1 => {
            admin.remove_artifact(&owned_artifact(evidence))?;
            Ok(())
        }
        2 => {
            admin.cleanup_standard_lock(&owned_lock(evidence, kind))?;
            Ok(())
        }
        _ => Err("Journaled owned artifact has an invalid link count".to_string()),
    }
}

fn cleanup_private(app_data_dir: &Path, record: &GitJournalRecord) -> Result<(), String> {
    let app_data = app_data_dir
        .canonicalize()
        .map_err(|error| format!("failed to resolve cleanup app-data: {error}"))?;
    let expected_parent = app_data.join("code").join("git-candidates");
    let evidence = [
        Some((&record.artifacts.candidate_index, candidate_label(record))),
        record
            .artifacts
            .source
            .as_ref()
            .map(|source| (source, "stage-source")),
        record
            .artifacts
            .message
            .as_ref()
            .map(|message| (message, "commit-message")),
    ];
    for (artifact, _) in evidence.iter().flatten() {
        if Path::new(&artifact.parent_path) != expected_parent {
            return Err("Journal private artifact escaped its exact cleanup root".to_string());
        }
    }
    match fs::symlink_metadata(&expected_parent) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "failed to inspect private Git cleanup directory: {error}"
            ));
        }
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err("Private Git cleanup directory was replaced".to_string());
        }
        Ok(_) => {}
    }
    let store = PrivateArtifactStore::for_mutation(&app_data)?;
    // Remove source/message before the candidate. Every call is independently
    // absent-or-exact so a crash between entries is restartable.
    for (artifact, label) in [evidence[1], evidence[2], evidence[0]]
        .into_iter()
        .flatten()
    {
        store.remove_if_absent_or_exact(&private_identity(artifact, label)?)?;
    }
    Ok(())
}

fn candidate_label(record: &GitJournalRecord) -> &'static str {
    match record.operation {
        CodeGitOperation::Stage | CodeGitOperation::Unstage => "candidate-index",
        CodeGitOperation::Commit => "commit-index",
    }
}

fn pin_recorded_admin(record: &GitJournalRecord) -> Result<PinnedAdminDirectory, String> {
    let admin = PinnedAdminDirectory::pin(Path::new(&record.repository.admin.exact_path))?;
    require_owned_admin(&admin, &record.repository.admin)?;
    Ok(admin)
}

fn publish_evidence(
    record: &GitJournalRecord,
) -> Result<
    (
        &GitJournalArtifactEvidence,
        StandardLockKind,
        &GitJournalArtifactEvidence,
        StandardLockKind,
    ),
    String,
> {
    let index = record
        .artifacts
        .index_artifact
        .as_ref()
        .ok_or_else(|| "Published recovery is missing its index evidence".to_string())?;
    let head = record
        .artifacts
        .head_artifact
        .as_ref()
        .ok_or_else(|| "Published recovery is missing its HEAD evidence".to_string())?;
    if index.link_count != 2 || head.link_count != 2 {
        return Err("Published recovery requires durable two-link evidence".to_string());
    }
    Ok(match record.operation {
        CodeGitOperation::Stage | CodeGitOperation::Unstage => {
            (index, StandardLockKind::Index, head, StandardLockKind::Head)
        }
        CodeGitOperation::Commit => (head, StandardLockKind::Head, index, StandardLockKind::Index),
    })
}

fn collect_cleanup<const N: usize>(results: [Result<(), String>; N]) -> Result<(), String> {
    let errors = results
        .into_iter()
        .filter_map(Result::err)
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}
