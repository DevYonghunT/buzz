use std::path::Path;

#[cfg(all(test, unix))]
mod crash_tests;
pub(super) mod fault;
mod recovery;
pub(super) use recovery::{complete_record_cleanup, recover_record};

use fault::{checkpoint, TransactionFaultBoundary};

use super::engine::{digest_bytes, random_hex, RepoProjection};
use super::git_command::{DirectoryIdentity, FileIdentity, GitCommitIdentity, GitWriteCommand};
use super::journal::{
    GitJournalArtifactEvidence, GitJournalArtifactSet, GitJournalBindingKey,
    GitJournalCommitTransaction, GitJournalFileIdentity, GitJournalIndexTransaction,
    GitJournalPathIdentity, GitJournalPhase, GitJournalRecord, GitJournalRepositoryEvidence,
    GitJournalRequestCoordinate, GitJournalTimestamp, GitOperationJournalStore,
};
use super::owned_lock::{
    LockAcquisition, OwnedArtifact, OwnedDirectoryIdentity, OwnedFileIdentity, OwnedStandardLock,
    PinnedAdminDirectory, StandardLockKind,
};
use super::private_artifact::PrivateArtifactStore;
use super::protocol::{
    CodeGitCommitIdentity, CodeGitCommitReceipt, CodeGitIndexMutationReceipt,
    CodeGitMutationReceipt, CodeGitOperation,
};
use super::repository::{
    inspect_repository, output_line_for_transaction, prepare_commit_inputs,
    prepare_index_candidate, revalidate_projection_evidence,
};
use crate::code_workspace::CodeThreadBinding;

pub(super) struct IndexMutationClaim<'a> {
    pub(super) app_data_dir: &'a Path,
    pub(super) binding: &'a CodeThreadBinding,
    pub(super) key: GitJournalBindingKey,
    pub(super) input_digest: String,
    pub(super) write_generation: u64,
    pub(super) snapshot_id: String,
    pub(super) file_id: String,
    pub(super) path: String,
    pub(super) operation: CodeGitOperation,
}

pub(super) struct CommitClaim<'a> {
    pub(super) app_data_dir: &'a Path,
    pub(super) binding: &'a CodeThreadBinding,
    pub(super) key: GitJournalBindingKey,
    pub(super) input_digest: String,
    pub(super) write_generation: u64,
    pub(super) snapshot_id: String,
    pub(super) identity: CodeGitCommitIdentity,
    pub(super) message: String,
    pub(super) projection: RepoProjection,
}

pub(super) fn execute_index(
    claim: IndexMutationClaim<'_>,
    projection: RepoProjection,
) -> Result<CodeGitIndexMutationReceipt, String> {
    let operation_id = random_hex()?;
    let journal_store = GitOperationJournalStore::for_app_data(claim.app_data_dir)?;
    ensure_clear(&journal_store, &claim.key)?;
    let private = PrivateArtifactStore::for_mutation(claim.app_data_dir)?;
    let stage = claim.operation == CodeGitOperation::Stage;
    let prepared = prepare_index_candidate(&private, &projection, &claim.path, stage)?;
    let second_projection = inspect_repository(claim.binding)?;
    if second_projection.preimage != projection.preimage {
        cleanup_private_preclaim(
            &private,
            &prepared.candidate,
            prepared.source.as_ref(),
            None,
        );
        return Err(
            "Git workspace changed while the private candidate index was prepared".to_string(),
        );
    }
    let admin = PinnedAdminDirectory::pin(&projection.admin)?;
    ensure_admin_identity(&admin, &projection.admin_identity)?;
    let index_name = admin.issue_artifact_component()?;
    let head_name = admin.issue_artifact_component()?;
    let mut record = GitJournalRecord {
        record_id: random_hex()?,
        operation_id: operation_id.clone(),
        key: claim.key.clone(),
        operation: claim.operation,
        phase: GitJournalPhase::Prepared,
        request: GitJournalRequestCoordinate {
            write_generation: claim.write_generation,
            snapshot_id: claim.snapshot_id.clone(),
            file_id: Some(claim.file_id.clone()),
        },
        input_digest: claim.input_digest,
        repository: repository_evidence(&projection),
        artifacts: GitJournalArtifactSet {
            candidate_index: private_evidence(&prepared.candidate)?,
            source: prepared.source.as_ref().map(private_evidence).transpose()?,
            message: None,
            index_artifact_name: index_name.component().to_string(),
            head_artifact_name: head_name.component().to_string(),
            index_artifact: None,
            head_artifact: None,
        },
        index_transaction: Some(GitJournalIndexTransaction {
            expected_index_digest: prepared.expected_index_digest.clone(),
            expected_semantic_digest: prepared.expected_semantic_digest,
            selected_path: claim.path,
            selected_mode: prepared.selected_mode.clone(),
            selected_blob_oid: prepared.selected_blob_oid.clone(),
        }),
        commit_transaction: None,
        receipt: None,
        acknowledgement: None,
        diagnostic: None,
        recovery_started: false,
        cleanup_complete: false,
    };
    append_record(&journal_store, &record)?;
    checkpoint(TransactionFaultBoundary::PreparedPersisted)?;

    let mut publish_outcome_unknown = false;
    let result = (|| {
        if let Some(source) = &prepared.source {
            let oid = projection.runner.run(GitWriteCommand::HashObject {
                write: true,
                source: source.clone(),
            })?;
            if output_line_for_transaction(&oid)? != prepared.selected_blob_oid {
                return Err("Git wrote a blob with an unexpected object id".to_string());
            }
            checkpoint(TransactionFaultBoundary::BlobObjectWritten)?;
            record.phase = GitJournalPhase::ObjectWritten;
            replace_record(&journal_store, &record)?;
            checkpoint(TransactionFaultBoundary::ObjectWrittenPersisted)?;
        }

        let candidate_bytes = private.read(&prepared.candidate)?;
        let index_artifact = admin.create_artifact(&index_name, &candidate_bytes)?;
        checkpoint(TransactionFaultBoundary::IndexArtifactDurable)?;
        let head_artifact =
            admin.create_artifact(&head_name, format!("{}\n", projection.head).as_bytes())?;
        checkpoint(TransactionFaultBoundary::HeadArtifactDurable)?;
        record.artifacts.index_artifact =
            Some(owned_artifact_evidence(&projection.admin, &index_artifact));
        record.artifacts.head_artifact =
            Some(owned_artifact_evidence(&projection.admin, &head_artifact));
        record.phase = GitJournalPhase::ArtifactsReady;
        replace_record(&journal_store, &record)?;
        checkpoint(TransactionFaultBoundary::ArtifactsReadyPersisted)?;

        let index_lock = acquire(&admin, &index_artifact, StandardLockKind::Index)?;
        checkpoint(TransactionFaultBoundary::IndexLockDurable)?;
        let head_lock = match acquire(&admin, &head_artifact, StandardLockKind::Head) {
            Ok(lock) => lock,
            Err(error) => {
                cleanup_unpublished(
                    &admin,
                    Some(&index_lock),
                    None,
                    &index_artifact,
                    &head_artifact,
                );
                return Err(error);
            }
        };
        checkpoint(TransactionFaultBoundary::HeadLockDurable)?;
        admin.revalidate_standard_lock(&index_lock)?;
        admin.revalidate_standard_lock(&head_lock)?;
        record.artifacts.index_artifact = Some(owned_lock_evidence(&projection.admin, &index_lock));
        record.artifacts.head_artifact = Some(owned_lock_evidence(&projection.admin, &head_lock));
        record.phase = GitJournalPhase::LocksReady;
        replace_record(&journal_store, &record)?;
        checkpoint(TransactionFaultBoundary::LocksReadyPersisted)?;

        checkpoint(TransactionFaultBoundary::BeforeIndexPublish)?;
        let _published = match admin.publish_after_cas(&index_lock, || {
            revalidate_projection_evidence(&projection)?;
            admin.revalidate_standard_lock(&head_lock)
        }) {
            Ok(published) => published,
            Err(failure) => {
                publish_outcome_unknown = failure.outcome_unknown();
                return Err(failure.into_message());
            }
        };
        checkpoint(TransactionFaultBoundary::IndexPublishDurable)?;
        record.phase = GitJournalPhase::IndexPublished;
        replace_record(&journal_store, &record)?;
        checkpoint(TransactionFaultBoundary::IndexPublishedPersisted)?;

        let receipt = CodeGitIndexMutationReceipt {
            operation_id: operation_id.clone(),
            operation: claim.operation,
            scope: claim.key.scope.clone(),
            thread_id: claim.key.thread_id.clone(),
            request_generation: claim.write_generation,
            before_snapshot_id: claim.snapshot_id,
            file_id: claim.file_id,
            disposition: if stage { "staged" } else { "unstaged" }.to_string(),
        };
        record.receipt = Some(CodeGitMutationReceipt::Index(receipt.clone()));
        record.phase = GitJournalPhase::CompletedAwaitingAck;
        record.cleanup_complete = false;
        replace_record(&journal_store, &record)?;
        checkpoint(TransactionFaultBoundary::CompletedReceiptPersisted)?;
        recovery::complete_cleanup(claim.app_data_dir, &journal_store, &mut record)?;
        checkpoint(TransactionFaultBoundary::CleanupCompleted)?;
        checkpoint(TransactionFaultBoundary::ResponseReady)?;
        Ok(receipt)
    })();
    if publish_outcome_unknown {
        result
    } else {
        finish_or_mark_uncertain(claim.app_data_dir, &journal_store, &mut record, result)
    }
}

pub(super) fn execute_commit(claim: CommitClaim<'_>) -> Result<CodeGitCommitReceipt, String> {
    let operation_id = random_hex()?;
    let journal_store = GitOperationJournalStore::for_app_data(claim.app_data_dir)?;
    ensure_clear(&journal_store, &claim.key)?;
    let private = PrivateArtifactStore::for_mutation(claim.app_data_dir)?;
    let mut prepared = prepare_commit_inputs(&private, &claim.projection, &claim.message)?;
    let second = inspect_repository(claim.binding)?;
    if second.preimage != claim.projection.preimage {
        cleanup_private_preclaim(&private, &prepared.candidate, None, Some(&prepared.message));
        return Err("Git workspace changed while commit inputs were frozen".to_string());
    }
    let admin = PinnedAdminDirectory::pin(&claim.projection.admin)?;
    ensure_admin_identity(&admin, &claim.projection.admin_identity)?;
    let index_name = admin.issue_artifact_component()?;
    let head_name = admin.issue_artifact_component()?;
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "System clock is before the Unix epoch".to_string())?
        .as_secs()
        .try_into()
        .map_err(|_| "System clock is outside the supported Git range".to_string())?;
    let frozen_timestamp = GitJournalTimestamp {
        unix_seconds: timestamp,
        offset_minutes: 0,
    };
    let mut record = GitJournalRecord {
        record_id: random_hex()?,
        operation_id: operation_id.clone(),
        key: claim.key.clone(),
        operation: CodeGitOperation::Commit,
        phase: GitJournalPhase::Prepared,
        request: GitJournalRequestCoordinate {
            write_generation: claim.write_generation,
            snapshot_id: claim.snapshot_id.clone(),
            file_id: None,
        },
        input_digest: claim.input_digest,
        repository: repository_evidence(&claim.projection),
        artifacts: GitJournalArtifactSet {
            candidate_index: private_evidence(&prepared.candidate)?,
            source: None,
            message: Some(private_evidence(&prepared.message)?),
            index_artifact_name: index_name.component().to_string(),
            head_artifact_name: head_name.component().to_string(),
            index_artifact: None,
            head_artifact: None,
        },
        index_transaction: None,
        commit_transaction: Some(GitJournalCommitTransaction {
            identity: claim.identity.clone(),
            author_timestamp: frozen_timestamp,
            committer_timestamp: frozen_timestamp,
            index_semantic_digest: claim.projection.index_semantic_digest.clone(),
            message_digest: prepared.message.digest.clone(),
            tree_oid: None,
            commit_oid: None,
        }),
        receipt: None,
        acknowledgement: None,
        diagnostic: None,
        recovery_started: false,
        cleanup_complete: false,
    };
    append_record(&journal_store, &record)?;
    checkpoint(TransactionFaultBoundary::PreparedPersisted)?;

    let mut publish_outcome_unknown = false;
    let result = (|| {
        let tree_output = claim.projection.runner.run(GitWriteCommand::WriteTree {
            index: prepared.candidate.clone(),
        })?;
        let tree = output_line_for_transaction(&tree_output)?.to_string();
        let secured = private.secure_after_mutation(Path::new(&prepared.candidate.path))?;
        let entries = claim
            .projection
            .runner
            .run(GitWriteCommand::ListStageEntries {
                index: Some(secured.clone()),
            })?;
        if digest_bytes(&entries.stdout) != claim.projection.index_semantic_digest {
            return Err("Frozen commit index semantics changed while writing its tree".to_string());
        }
        verify_object(&claim.projection, &tree, "tree")?;
        checkpoint(TransactionFaultBoundary::TreeObjectWritten)?;
        record.artifacts.candidate_index = private_evidence(&secured)?;
        commit_evidence_mut(&mut record)?.tree_oid = Some(tree.clone());
        replace_record(&journal_store, &record)?;
        checkpoint(TransactionFaultBoundary::TreeEvidencePersisted)?;
        prepared.candidate = secured;

        let timestamp = format!("{} +0000", frozen_timestamp.unix_seconds);
        let commit_output = claim.projection.runner.run(GitWriteCommand::CommitTree {
            tree: tree.clone(),
            parent: claim.projection.head.clone(),
            identity: GitCommitIdentity {
                name: claim.identity.name.clone(),
                email: claim.identity.email.clone(),
            },
            timestamp,
            message: prepared.message.clone(),
        })?;
        let commit_oid = output_line_for_transaction(&commit_output)?.to_string();
        verify_object(&claim.projection, &commit_oid, "commit")?;
        checkpoint(TransactionFaultBoundary::CommitObjectWritten)?;
        commit_evidence_mut(&mut record)?.commit_oid = Some(commit_oid.clone());
        record.phase = GitJournalPhase::ObjectWritten;
        replace_record(&journal_store, &record)?;
        checkpoint(TransactionFaultBoundary::ObjectWrittenPersisted)?;

        let index_bytes = private
            .read(&prepared.candidate)
            .map_err(|error| format!("failed to reread frozen commit index: {error}"))?;
        let index_artifact = admin.create_artifact(&index_name, &index_bytes)?;
        checkpoint(TransactionFaultBoundary::IndexArtifactDurable)?;
        let head_artifact =
            admin.create_artifact(&head_name, format!("{commit_oid}\n").as_bytes())?;
        checkpoint(TransactionFaultBoundary::HeadArtifactDurable)?;
        record.artifacts.index_artifact = Some(owned_artifact_evidence(
            &claim.projection.admin,
            &index_artifact,
        ));
        record.artifacts.head_artifact = Some(owned_artifact_evidence(
            &claim.projection.admin,
            &head_artifact,
        ));
        record.phase = GitJournalPhase::ArtifactsReady;
        replace_record(&journal_store, &record)?;
        checkpoint(TransactionFaultBoundary::ArtifactsReadyPersisted)?;

        let index_lock = acquire(&admin, &index_artifact, StandardLockKind::Index)?;
        checkpoint(TransactionFaultBoundary::IndexLockDurable)?;
        let head_lock = match acquire(&admin, &head_artifact, StandardLockKind::Head) {
            Ok(lock) => lock,
            Err(error) => {
                cleanup_unpublished(
                    &admin,
                    Some(&index_lock),
                    None,
                    &index_artifact,
                    &head_artifact,
                );
                return Err(error);
            }
        };
        checkpoint(TransactionFaultBoundary::HeadLockDurable)?;
        record.artifacts.index_artifact =
            Some(owned_lock_evidence(&claim.projection.admin, &index_lock));
        record.artifacts.head_artifact =
            Some(owned_lock_evidence(&claim.projection.admin, &head_lock));
        record.phase = GitJournalPhase::LocksReady;
        replace_record(&journal_store, &record)?;
        checkpoint(TransactionFaultBoundary::LocksReadyPersisted)?;

        checkpoint(TransactionFaultBoundary::BeforeHeadPublish)?;
        let _published = match admin.publish_after_cas(&head_lock, || {
            revalidate_projection_evidence(&claim.projection)?;
            admin.revalidate_standard_lock(&index_lock)
        }) {
            Ok(published) => published,
            Err(failure) => {
                publish_outcome_unknown = failure.outcome_unknown();
                return Err(failure.into_message());
            }
        };
        checkpoint(TransactionFaultBoundary::HeadPublishDurable)?;
        record.phase = GitJournalPhase::HeadPublished;
        replace_record(&journal_store, &record)?;
        checkpoint(TransactionFaultBoundary::HeadPublishedPersisted)?;

        let receipt = CodeGitCommitReceipt {
            operation_id: operation_id.clone(),
            operation: CodeGitOperation::Commit,
            scope: claim.key.scope.clone(),
            thread_id: claim.key.thread_id.clone(),
            request_generation: claim.write_generation,
            before_snapshot_id: claim.snapshot_id,
            previous_head: claim.projection.head.clone(),
            commit: commit_oid,
            tree,
            disposition: "committed".to_string(),
        };
        record.receipt = Some(CodeGitMutationReceipt::Commit(receipt.clone()));
        record.phase = GitJournalPhase::CompletedAwaitingAck;
        record.cleanup_complete = false;
        replace_record(&journal_store, &record)?;
        checkpoint(TransactionFaultBoundary::CompletedReceiptPersisted)?;
        recovery::complete_cleanup(claim.app_data_dir, &journal_store, &mut record)?;
        checkpoint(TransactionFaultBoundary::CleanupCompleted)?;
        checkpoint(TransactionFaultBoundary::ResponseReady)?;
        Ok(receipt)
    })();
    if publish_outcome_unknown {
        result
    } else {
        finish_or_mark_uncertain(claim.app_data_dir, &journal_store, &mut record, result)
    }
}

fn verify_object(projection: &RepoProjection, oid: &str, expected: &str) -> Result<(), String> {
    let output = projection.runner.run(GitWriteCommand::ObjectType {
        oid: oid.to_string(),
    })?;
    if output_line_for_transaction(&output)? != expected {
        return Err(format!("Git object {oid} is not a {expected}"));
    }
    Ok(())
}

fn acquire(
    admin: &PinnedAdminDirectory,
    artifact: &OwnedArtifact,
    kind: StandardLockKind,
) -> Result<OwnedStandardLock, String> {
    match admin.acquire_standard_lock(artifact, kind)? {
        LockAcquisition::Acquired(lock) | LockAcquisition::AlreadyOwned(lock) => Ok(lock),
        LockAcquisition::Foreign => Err(format!(
            "Git {} is busy; a foreign lock was preserved",
            if kind == StandardLockKind::Index {
                "index"
            } else {
                "HEAD"
            }
        )),
    }
}

fn ensure_clear(
    store: &GitOperationJournalStore,
    key: &GitJournalBindingKey,
) -> Result<(), String> {
    if store.load()?.blocking_keys().contains(key) {
        Err("Another Git operation must be reconciled or acknowledged first".to_string())
    } else {
        Ok(())
    }
}

fn append_record(
    store: &GitOperationJournalStore,
    record: &GitJournalRecord,
) -> Result<(), String> {
    let mut journal = store.load()?;
    if journal.blocking_keys().contains(&record.key) {
        return Err("Another Git operation must be reconciled or acknowledged first".to_string());
    }
    journal.records.push(record.clone());
    store.save(&journal)
}

fn replace_record(
    store: &GitOperationJournalStore,
    record: &GitJournalRecord,
) -> Result<(), String> {
    let mut journal = store.load()?;
    let current = journal
        .records
        .iter_mut()
        .find(|current| current.record_id == record.record_id)
        .ok_or_else(|| "Durable Git transaction disappeared".to_string())?;
    *current = record.clone();
    store.save(&journal)
}

fn finish_or_mark_uncertain<T>(
    app_data_dir: &Path,
    store: &GitOperationJournalStore,
    record: &mut GitJournalRecord,
    result: Result<T, String>,
) -> Result<T, String> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => {
            if matches!(
                record.phase,
                GitJournalPhase::IndexPublished
                    | GitJournalPhase::HeadPublished
                    | GitJournalPhase::CompletedAwaitingAck
            ) {
                return Err(error);
            }
            if record.phase != GitJournalPhase::Uncertain {
                let diagnostic = match recovery::cleanup_before_uncertain(app_data_dir, record) {
                    Ok(()) => sanitize_diagnostic(&error),
                    Err(cleanup_error) => sanitize_diagnostic(&format!(
                        "{error}; foreign or replaced cleanup evidence was preserved: {cleanup_error}"
                    )),
                };
                record.phase = GitJournalPhase::Uncertain;
                record.receipt = None;
                record.acknowledgement = None;
                record.diagnostic = Some(diagnostic);
                if let Err(save_error) = replace_record(store, record) {
                    return Err(format!(
                        "{error}; failed to durably mark Git transaction uncertain: {save_error}"
                    ));
                }
            }
            Err(error)
        }
    }
}

fn sanitize_diagnostic(value: &str) -> String {
    let value = value.trim();
    let value = if value.is_empty() {
        "Native Git evidence became uncertain"
    } else {
        value
    };
    let mut end = value.len().min(4_096);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value[..end].trim().to_string()
}

fn repository_evidence(projection: &RepoProjection) -> GitJournalRepositoryEvidence {
    GitJournalRepositoryEvidence {
        repository_identity: projection.repository_identity.clone(),
        root: directory_evidence(&projection.root_identity),
        admin: directory_evidence(&projection.admin_identity),
        common_dir: directory_evidence(&projection.common_identity),
        object_database: directory_evidence(&projection.object_database_identity),
        worktree_git_file: file_path_evidence(&projection.worktree_git_file),
        git_executable: file_path_evidence(&projection.git_identity),
        previous_head: projection.head.clone(),
        head_file_digest: projection.head_file_digest.clone(),
        before_index_digest: projection.index_digest.clone(),
        before_config_digest: projection.config_digest.clone(),
    }
}

fn directory_evidence(value: &DirectoryIdentity) -> GitJournalPathIdentity {
    GitJournalPathIdentity {
        exact_path: value.path.clone(),
        device: value.device,
        inode: value.inode,
        owner: value.owner,
        mode: value.mode,
        link_count: value.link_count,
    }
}

fn file_path_evidence(value: &FileIdentity) -> GitJournalFileIdentity {
    GitJournalFileIdentity {
        exact_path: value.path.clone(),
        device: value.device,
        inode: value.inode,
        owner: value.owner,
        mode: value.mode,
        link_count: value.link_count,
        size: value.size,
        sha256: value.digest.clone(),
    }
}

fn private_evidence(value: &FileIdentity) -> Result<GitJournalArtifactEvidence, String> {
    let path = Path::new(&value.path);
    let parent = path
        .parent()
        .and_then(Path::to_str)
        .ok_or_else(|| "Private Git artifact parent is not UTF-8".to_string())?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Private Git artifact name is not UTF-8".to_string())?;
    Ok(GitJournalArtifactEvidence {
        parent_path: parent.to_string(),
        name: name.to_string(),
        device: value.device,
        inode: value.inode,
        owner: value.owner,
        mode: value.mode,
        link_count: value.link_count,
        size: value.size,
        sha256: value.digest.clone(),
    })
}

fn owned_artifact_evidence(admin: &Path, value: &OwnedArtifact) -> GitJournalArtifactEvidence {
    owned_evidence(admin, &value.component, &value.identity)
}

fn owned_lock_evidence(admin: &Path, value: &OwnedStandardLock) -> GitJournalArtifactEvidence {
    owned_evidence(admin, &value.artifact_component, &value.identity)
}

fn owned_evidence(
    admin: &Path,
    component: &str,
    identity: &OwnedFileIdentity,
) -> GitJournalArtifactEvidence {
    GitJournalArtifactEvidence {
        parent_path: admin.to_string_lossy().to_string(),
        name: component.to_string(),
        device: identity.device,
        inode: identity.inode,
        owner: identity.owner,
        mode: identity.mode,
        link_count: identity.link_count,
        size: identity.size,
        sha256: identity.sha256.clone(),
    }
}

fn ensure_admin_identity(
    admin: &PinnedAdminDirectory,
    expected: &DirectoryIdentity,
) -> Result<(), String> {
    let actual: &OwnedDirectoryIdentity = admin.identity();
    if actual.path != expected.path
        || actual.device != expected.device
        || actual.inode != expected.inode
        || actual.owner != expected.owner
        || actual.mode != expected.mode
    {
        return Err("Pinned Git admin identity changed before journal claim".to_string());
    }
    Ok(())
}

fn commit_evidence_mut(
    record: &mut GitJournalRecord,
) -> Result<&mut GitJournalCommitTransaction, String> {
    record
        .commit_transaction
        .as_mut()
        .ok_or_else(|| "Commit journal lost its transaction evidence".to_string())
}

fn cleanup_private_preclaim(
    store: &PrivateArtifactStore,
    candidate: &FileIdentity,
    source: Option<&FileIdentity>,
    message: Option<&FileIdentity>,
) {
    if let Some(value) = source {
        let _ = store.remove(value);
    }
    if let Some(value) = message {
        let _ = store.remove(value);
    }
    let _ = store.remove(candidate);
}

fn cleanup_unpublished(
    admin: &PinnedAdminDirectory,
    index_lock: Option<&OwnedStandardLock>,
    head_lock: Option<&OwnedStandardLock>,
    index_artifact: &OwnedArtifact,
    head_artifact: &OwnedArtifact,
) {
    let index = index_lock
        .and_then(|lock| admin.release_standard_lock(lock).ok())
        .unwrap_or_else(|| index_artifact.clone());
    let head = head_lock
        .and_then(|lock| admin.release_standard_lock(lock).ok())
        .unwrap_or_else(|| head_artifact.clone());
    let _ = admin.remove_artifact(&head);
    let _ = admin.remove_artifact(&index);
}
