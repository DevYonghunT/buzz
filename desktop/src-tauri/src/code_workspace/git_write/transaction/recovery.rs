//! Strict live-evidence recovery for one durable Git write transaction.

use std::fs;
use std::path::Path;

use super::super::engine::digest_bytes;
use super::super::git_command::{FileIdentity, GitCommitIdentity, GitWriteCommand};
use super::super::journal::{
    GitJournalArtifactEvidence, GitJournalPhase, GitJournalRecord, GitOperationJournalStore,
};
use super::super::owned_lock::{
    IssuedArtifactComponent, LockAcquisition, OwnedArtifact, OwnedFileIdentity, OwnedLockPlacement,
    OwnedStandardLock, PinnedAdminDirectory, StandardLockKind,
};
use super::super::private_artifact::PrivateArtifactStore;
use super::super::protocol::{
    CodeGitCommitReceipt, CodeGitIndexMutationReceipt, CodeGitMutationReceipt, CodeGitOperation,
};
use super::super::repository::output_line_for_transaction;
use crate::code_workspace::{CodeExecutionMode, CodeThreadBinding};

mod cleanup;
mod repository;
#[cfg(test)]
mod tests;

use cleanup::{cleanup_after_publish, prove_expected_publish};
use repository::RecoveryRepository;

pub(in crate::code_workspace::git_write::transaction) use cleanup::cleanup_before_uncertain;

const MAX_DIAGNOSTIC_BYTES: usize = 4 * 1024;

enum ActiveRecoveryError {
    Ordinary(String),
    PublishOutcomeUnknown(String),
}

impl From<String> for ActiveRecoveryError {
    fn from(error: String) -> Self {
        Self::Ordinary(error)
    }
}

type ActiveRecoveryResult<T> = Result<T, ActiveRecoveryError>;

/// Recover exactly one strict journal record, converging to completed or uncertain.
pub(in crate::code_workspace::git_write) fn recover_record(
    app_data_dir: &Path,
    binding: &CodeThreadBinding,
    record_id: &str,
) -> Result<(), String> {
    let store = GitOperationJournalStore::for_app_data(app_data_dir)?;
    let journal = store.load()?;
    let mut record = journal
        .records
        .iter()
        .find(|record| record.record_id == record_id)
        .cloned()
        .ok_or_else(|| "Git recovery record does not exist".to_string())?;
    validate_binding(binding, &record)?;
    match record.phase {
        GitJournalPhase::CompletedAwaitingAck => {
            return super::super::with_git_authority(|| {
                complete_cleanup(app_data_dir, &store, &mut record)
            });
        }
        GitJournalPhase::Acknowledged | GitJournalPhase::Uncertain => return Ok(()),
        _ => {}
    }
    super::super::with_git_authority(|| {
        if !record.recovery_started {
            transition(&store, &mut record, |next| next.recovery_started = true)?;
        }
        if is_durable_publish(&record) {
            return finalize(app_data_dir, &store, &mut record);
        }

        let result = recover_active(app_data_dir, &store, &mut record, binding);
        match result {
            Ok(()) => Ok(()),
            Err(ActiveRecoveryError::PublishOutcomeUnknown(error)) => Err(error),
            Err(ActiveRecoveryError::Ordinary(error))
                if matches!(
                    record.phase,
                    GitJournalPhase::IndexPublished | GitJournalPhase::HeadPublished
                ) =>
            {
                Err(error)
            }
            Err(ActiveRecoveryError::Ordinary(error)) => {
                let diagnostic = match cleanup_before_uncertain(app_data_dir, &record) {
                    Ok(()) => sanitize_diagnostic(&error),
                    Err(cleanup_error) => sanitize_diagnostic(&format!(
                        "{error}; foreign or replaced cleanup evidence was preserved: {cleanup_error}"
                    )),
                };
                transition(&store, &mut record, |next| {
                    next.phase = GitJournalPhase::Uncertain;
                    next.receipt = None;
                    next.acknowledgement = None;
                    next.diagnostic = Some(diagnostic);
                })?;
                Ok(())
            }
        }
    })
}

/// Re-enter only the restartable cleanup of a durable completed receipt.
pub(in crate::code_workspace::git_write) fn complete_record_cleanup(
    app_data_dir: &Path,
    record_id: &str,
) -> Result<(), String> {
    let store = GitOperationJournalStore::for_app_data(app_data_dir)?;
    let journal = store.load()?;
    let mut record = journal
        .records
        .iter()
        .find(|record| record.record_id == record_id)
        .cloned()
        .ok_or_else(|| "Git cleanup record does not exist".to_string())?;
    match record.phase {
        GitJournalPhase::CompletedAwaitingAck => {
            super::super::with_git_authority(|| complete_cleanup(app_data_dir, &store, &mut record))
        }
        GitJournalPhase::Acknowledged if record.cleanup_complete => Ok(()),
        _ => Err("Git record is not ready for completed cleanup".to_string()),
    }
}

fn recover_active(
    app_data_dir: &Path,
    store: &GitOperationJournalStore,
    record: &mut GitJournalRecord,
    binding: &CodeThreadBinding,
) -> ActiveRecoveryResult<()> {
    let repository = RecoveryRepository::open(binding, record)?;
    match record.operation {
        CodeGitOperation::Stage | CodeGitOperation::Unstage => {
            recover_index(app_data_dir, store, record, &repository)
        }
        CodeGitOperation::Commit => recover_commit(app_data_dir, store, record, &repository),
    }
}

fn recover_index(
    app_data_dir: &Path,
    store: &GitOperationJournalStore,
    record: &mut GitJournalRecord,
    repository: &RecoveryRepository,
) -> ActiveRecoveryResult<()> {
    let transaction = record
        .index_transaction
        .as_ref()
        .ok_or_else(|| "Index recovery is missing immutable transaction evidence".to_string())?;
    if repository.index_digest == transaction.expected_index_digest
        && repository.head == record.repository.previous_head
    {
        require_publishable_evidence(record, GitJournalPhase::LocksReady)?;
        prove_expected_publish(record, &repository.admin)?;
        persist_publish(store, record, GitJournalPhase::IndexPublished)?;
        return Ok(finalize(app_data_dir, store, record)?);
    }
    if repository.index_digest != record.repository.before_index_digest
        || repository.head != record.repository.previous_head
    {
        return Err("Live index or HEAD is a third state during index recovery"
            .to_string()
            .into());
    }
    repository.revalidate_for_resume(record)?;
    let private = PrivateRecovery::open_exact(app_data_dir, record)?;
    ensure_index_object(store, record, repository, &private)?;
    let (index_artifact, head_artifact) = ensure_artifacts(store, record, repository, &private)?;
    let (index, head) = ensure_locks(store, record, repository, index_artifact, head_artifact)?;
    let (index_lock, head_lock) = match (index, head) {
        (LockPlacement::Published, LockPlacement::Locked(_)) => {
            persist_publish(store, record, GitJournalPhase::IndexPublished)?;
            return Ok(finalize(app_data_dir, store, record)?);
        }
        (LockPlacement::Locked(index), LockPlacement::Locked(head)) => (index, head),
        (_, LockPlacement::Published) => {
            return Err(
                "HEAD guard was unexpectedly published during index recovery"
                    .to_string()
                    .into(),
            )
        }
    };
    let _published = match repository.admin.publish_after_cas(&index_lock, || {
        repository.revalidate_for_resume(record)?;
        repository.admin.revalidate_standard_lock(&head_lock)
    }) {
        Ok(published) => published,
        Err(failure) if failure.outcome_unknown() => {
            return Err(ActiveRecoveryError::PublishOutcomeUnknown(
                failure.into_message(),
            ));
        }
        Err(failure) => return Err(failure.into_message().into()),
    };
    persist_publish(store, record, GitJournalPhase::IndexPublished)?;
    Ok(finalize(app_data_dir, store, record)?)
}

fn recover_commit(
    app_data_dir: &Path,
    store: &GitOperationJournalStore,
    record: &mut GitJournalRecord,
    repository: &RecoveryRepository,
) -> ActiveRecoveryResult<()> {
    let expected_commit = record
        .commit_transaction
        .as_ref()
        .and_then(|transaction| transaction.commit_oid.as_deref());
    if expected_commit.is_some_and(|expected| repository.head == expected)
        && repository.index_digest == record.repository.before_index_digest
    {
        require_publishable_evidence(record, GitJournalPhase::LocksReady)?;
        prove_expected_publish(record, &repository.admin)?;
        persist_publish(store, record, GitJournalPhase::HeadPublished)?;
        return Ok(finalize(app_data_dir, store, record)?);
    }
    if repository.head != record.repository.previous_head
        || repository.index_digest != record.repository.before_index_digest
    {
        return Err("Live HEAD or index is a third state during commit recovery"
            .to_string()
            .into());
    }
    repository.revalidate_for_resume(record)?;
    let mut private = PrivateRecovery::open_commit(app_data_dir, record, repository)?;
    ensure_commit_objects(store, record, repository, &mut private)?;
    let (index_artifact, head_artifact) = ensure_artifacts(store, record, repository, &private)?;
    let (index, head) = ensure_locks(store, record, repository, index_artifact, head_artifact)?;
    let (index_lock, head_lock) = match (index, head) {
        (LockPlacement::Locked(_), LockPlacement::Published) => {
            persist_publish(store, record, GitJournalPhase::HeadPublished)?;
            return Ok(finalize(app_data_dir, store, record)?);
        }
        (LockPlacement::Locked(index), LockPlacement::Locked(head)) => (index, head),
        (LockPlacement::Published, _) => {
            return Err(
                "Index guard was unexpectedly published during commit recovery"
                    .to_string()
                    .into(),
            )
        }
    };
    let _published = match repository.admin.publish_after_cas(&head_lock, || {
        repository.revalidate_for_resume(record)?;
        repository.admin.revalidate_standard_lock(&index_lock)
    }) {
        Ok(published) => published,
        Err(failure) if failure.outcome_unknown() => {
            return Err(ActiveRecoveryError::PublishOutcomeUnknown(
                failure.into_message(),
            ));
        }
        Err(failure) => return Err(failure.into_message().into()),
    };
    persist_publish(store, record, GitJournalPhase::HeadPublished)?;
    Ok(finalize(app_data_dir, store, record)?)
}

fn ensure_index_object(
    store: &GitOperationJournalStore,
    record: &mut GitJournalRecord,
    repository: &RecoveryRepository,
    private: &PrivateRecovery,
) -> Result<(), String> {
    let transaction = record
        .index_transaction
        .clone()
        .ok_or_else(|| "Index recovery lost transaction evidence".to_string())?;
    if record.operation == CodeGitOperation::Stage && transaction.selected_mode != "000000" {
        let source = private
            .source
            .clone()
            .ok_or_else(|| "Stage recovery lost its frozen source".to_string())?;
        let output = repository.runner.run(GitWriteCommand::HashObject {
            write: true,
            source,
        })?;
        if output_line_for_transaction(&output)? != transaction.selected_blob_oid {
            return Err("Recovered stage blob has an unexpected object id".to_string());
        }
        repository.verify_object(&transaction.selected_blob_oid, "blob")?;
        if record.phase == GitJournalPhase::Prepared {
            transition(store, record, |next| {
                next.phase = GitJournalPhase::ObjectWritten
            })?;
        }
    }
    Ok(())
}

fn ensure_commit_objects(
    store: &GitOperationJournalStore,
    record: &mut GitJournalRecord,
    repository: &RecoveryRepository,
    private: &mut PrivateRecovery,
) -> Result<(), String> {
    let mut transaction = record
        .commit_transaction
        .clone()
        .ok_or_else(|| "Commit recovery lost transaction evidence".to_string())?;
    if transaction.author_timestamp != transaction.committer_timestamp {
        return Err(
            "Commit recovery cannot reproduce distinct author and committer times".to_string(),
        );
    }
    verify_candidate_semantics(
        repository,
        &private.candidate,
        &transaction.index_semantic_digest,
    )?;
    let tree = if let Some(tree) = transaction.tree_oid.clone() {
        repository.verify_object(&tree, "tree")?;
        tree
    } else {
        let tree_output = repository.runner.run(GitWriteCommand::WriteTree {
            index: private.candidate.clone(),
        })?;
        let tree = output_line_for_transaction(&tree_output)?.to_string();
        let candidate = private
            .store
            .secure_after_mutation(Path::new(&private.candidate.path))?;
        verify_candidate_semantics(repository, &candidate, &transaction.index_semantic_digest)?;
        repository.verify_object(&tree, "tree")?;
        let candidate_evidence = private_file_evidence(&candidate)?;
        transition(store, record, |next| {
            next.artifacts.candidate_index = candidate_evidence;
            if let Some(commit) = &mut next.commit_transaction {
                commit.tree_oid = Some(tree.clone());
            }
        })?;
        transaction.tree_oid = Some(tree.clone());
        private.candidate = candidate;
        tree
    };
    let message = private
        .message
        .clone()
        .ok_or_else(|| "Commit recovery lost its canonical message".to_string())?;
    let commit = if let Some(commit) = transaction.commit_oid.clone() {
        repository.verify_object(&commit, "commit")?;
        commit
    } else {
        let timestamp = format_git_timestamp(transaction.committer_timestamp);
        let output = repository.runner.run(GitWriteCommand::CommitTree {
            tree: tree.clone(),
            parent: record.repository.previous_head.clone(),
            identity: GitCommitIdentity {
                name: transaction.identity.name.clone(),
                email: transaction.identity.email.clone(),
            },
            timestamp,
            message,
        })?;
        let commit = output_line_for_transaction(&output)?.to_string();
        repository.verify_object(&commit, "commit")?;
        commit
    };
    if transaction.commit_oid.is_none() || record.phase == GitJournalPhase::Prepared {
        transition(store, record, |next| {
            if let Some(evidence) = &mut next.commit_transaction {
                evidence.tree_oid = Some(tree);
                evidence.commit_oid = Some(commit);
            }
            next.phase = GitJournalPhase::ObjectWritten;
        })?;
    }
    Ok(())
}

fn verify_candidate_semantics(
    repository: &RecoveryRepository,
    candidate: &FileIdentity,
    expected: &str,
) -> Result<(), String> {
    let entries = repository.runner.run(GitWriteCommand::ListStageEntries {
        index: Some(candidate.clone()),
    })?;
    if digest_bytes(&entries.stdout) != expected {
        return Err("Recovered commit index has unexpected Git semantics".to_string());
    }
    Ok(())
}

fn ensure_artifacts(
    store: &GitOperationJournalStore,
    record: &mut GitJournalRecord,
    repository: &RecoveryRepository,
    private: &PrivateRecovery,
) -> Result<(OwnedArtifact, OwnedArtifact), String> {
    match (
        record.artifacts.index_artifact.as_ref(),
        record.artifacts.head_artifact.as_ref(),
    ) {
        (Some(index), Some(head)) => {
            return Ok((owned_artifact(index), owned_artifact(head)));
        }
        (None, None) => {}
        _ => return Err("Recovery journal contains partial owned artifact evidence".to_string()),
    }
    let index_name = IssuedArtifactComponent::from_journal(&record.artifacts.index_artifact_name)?;
    let head_name = IssuedArtifactComponent::from_journal(&record.artifacts.head_artifact_name)?;
    let candidate = private.store.read(&private.candidate)?;
    let head_bytes = match record.operation {
        CodeGitOperation::Stage | CodeGitOperation::Unstage => {
            format!("{}\n", record.repository.previous_head).into_bytes()
        }
        CodeGitOperation::Commit => {
            let commit = record
                .commit_transaction
                .as_ref()
                .and_then(|transaction| transaction.commit_oid.as_deref())
                .ok_or_else(|| "Commit recovery is missing its expected object id".to_string())?;
            format!("{commit}\n").into_bytes()
        }
    };
    let index = repository
        .admin
        .recover_or_create_artifact(&index_name, &candidate)?;
    let head = repository
        .admin
        .recover_or_create_artifact(&head_name, &head_bytes)?;
    let index_evidence = journal_artifact(&record.repository.admin.exact_path, &index);
    let head_evidence = journal_artifact(&record.repository.admin.exact_path, &head);
    transition(store, record, |next| {
        next.artifacts.index_artifact = Some(index_evidence);
        next.artifacts.head_artifact = Some(head_evidence);
        next.phase = GitJournalPhase::ArtifactsReady;
    })?;
    Ok((index, head))
}

fn ensure_locks(
    store: &GitOperationJournalStore,
    record: &mut GitJournalRecord,
    repository: &RecoveryRepository,
    index_artifact: OwnedArtifact,
    head_artifact: OwnedArtifact,
) -> Result<(LockPlacement, LockPlacement), String> {
    let index = ensure_lock(&repository.admin, index_artifact, StandardLockKind::Index)?;
    let head = match ensure_lock(&repository.admin, head_artifact, StandardLockKind::Head) {
        Ok(value) => value,
        Err(error) => {
            if let LockPlacement::Locked(lock) = &index {
                let _ = repository.admin.release_standard_lock(lock);
            }
            return Err(error);
        }
    };
    if let (LockPlacement::Locked(index), LockPlacement::Locked(head)) = (&index, &head) {
        let index_evidence = journal_lock(&record.repository.admin.exact_path, index);
        let head_evidence = journal_lock(&record.repository.admin.exact_path, head);
        if record.phase != GitJournalPhase::LocksReady
            || record.artifacts.index_artifact.as_ref() != Some(&index_evidence)
            || record.artifacts.head_artifact.as_ref() != Some(&head_evidence)
        {
            transition(store, record, |next| {
                next.artifacts.index_artifact = Some(index_evidence);
                next.artifacts.head_artifact = Some(head_evidence);
                next.phase = GitJournalPhase::LocksReady;
            })?;
        }
    }
    Ok((index, head))
}

enum LockPlacement {
    Locked(OwnedStandardLock),
    Published,
}

fn ensure_lock(
    admin: &PinnedAdminDirectory,
    artifact: OwnedArtifact,
    kind: StandardLockKind,
) -> Result<LockPlacement, String> {
    if artifact.identity.link_count == 1 {
        return match admin.acquire_standard_lock(&artifact, kind)? {
            LockAcquisition::Acquired(lock) | LockAcquisition::AlreadyOwned(lock) => {
                Ok(LockPlacement::Locked(lock))
            }
            LockAcquisition::Foreign => Err(format!(
                "Foreign {} was preserved",
                ensure_lock_kind_name(kind)
            )),
        };
    }
    let evidence = OwnedStandardLock {
        artifact_component: artifact.component,
        kind,
        identity: artifact.identity,
    };
    match admin.classify_owned_lock(&evidence)? {
        OwnedLockPlacement::Locked(lock) => Ok(LockPlacement::Locked(lock)),
        OwnedLockPlacement::Published(_) => Ok(LockPlacement::Published),
        OwnedLockPlacement::Released(artifact) => ensure_lock(admin, artifact, kind),
    }
}

struct PrivateRecovery {
    store: PrivateArtifactStore,
    candidate: FileIdentity,
    source: Option<FileIdentity>,
    message: Option<FileIdentity>,
}

impl PrivateRecovery {
    fn open_exact(app_data_dir: &Path, record: &GitJournalRecord) -> Result<Self, String> {
        Self::open(app_data_dir, record, None)
    }

    fn open_commit(
        app_data_dir: &Path,
        record: &GitJournalRecord,
        repository: &RecoveryRepository,
    ) -> Result<Self, String> {
        Self::open(app_data_dir, record, Some(repository))
    }

    fn open(
        app_data_dir: &Path,
        record: &GitJournalRecord,
        commit_repository: Option<&RecoveryRepository>,
    ) -> Result<Self, String> {
        let app_data = app_data_dir
            .canonicalize()
            .map_err(|error| format!("failed to resolve recovery app-data: {error}"))?;
        let expected_parent = app_data.join("code").join("git-candidates");
        if Path::new(&record.artifacts.candidate_index.parent_path) != expected_parent {
            return Err("Journal candidate escaped the private artifact directory".to_string());
        }
        let parent = fs::symlink_metadata(&expected_parent)
            .map_err(|error| format!("failed to inspect private recovery directory: {error}"))?;
        if parent.file_type().is_symlink() || !parent.is_dir() {
            return Err("Private recovery directory was replaced".to_string());
        }
        let store = PrivateArtifactStore::for_mutation(&app_data)?;
        let source = record
            .artifacts
            .source
            .as_ref()
            .map(|evidence| private_identity(evidence, "stage-source"))
            .transpose()?;
        if let Some(source) = &source {
            if store.refresh(Path::new(&source.path))? != *source {
                return Err("Frozen stage source no longer matches journal evidence".to_string());
            }
        }
        let message = record
            .artifacts
            .message
            .as_ref()
            .map(|evidence| private_identity(evidence, "commit-message"))
            .transpose()?;
        if let Some(message) = &message {
            if store.refresh(Path::new(&message.path))? != *message {
                return Err(
                    "Canonical commit message no longer matches journal evidence".to_string(),
                );
            }
        }
        let candidate_label = match record.operation {
            CodeGitOperation::Stage | CodeGitOperation::Unstage => "candidate-index",
            CodeGitOperation::Commit => "commit-index",
        };
        let expected = private_identity(&record.artifacts.candidate_index, candidate_label)?;
        let exact = store.refresh(Path::new(&expected.path));
        let candidate = match exact {
            Ok(observed) if observed == expected => expected,
            _ if can_adopt_commit_candidate(record, commit_repository) => {
                let repository = commit_repository
                    .ok_or_else(|| "Commit candidate adoption lost its repository".to_string())?;
                let adopted = store.secure_after_mutation(Path::new(&expected.path))?;
                let transaction = record.commit_transaction.as_ref().ok_or_else(|| {
                    "Commit candidate adoption lost transaction evidence".to_string()
                })?;
                verify_candidate_semantics(
                    repository,
                    &adopted,
                    &transaction.index_semantic_digest,
                )?;
                adopted
            }
            Ok(_) => return Err("Candidate index no longer matches journal evidence".to_string()),
            Err(error) => return Err(error),
        };
        Ok(Self {
            store,
            candidate,
            source,
            message,
        })
    }
}

fn can_adopt_commit_candidate(
    record: &GitJournalRecord,
    repository: Option<&RecoveryRepository>,
) -> bool {
    repository.is_some()
        && record.operation == CodeGitOperation::Commit
        && record.phase == GitJournalPhase::Prepared
        && record.artifacts.index_artifact.is_none()
        && record.artifacts.head_artifact.is_none()
        && record
            .commit_transaction
            .as_ref()
            .is_some_and(|transaction| {
                transaction.tree_oid.is_none() && transaction.commit_oid.is_none()
            })
        && record.artifacts.candidate_index.sha256 == record.repository.before_index_digest
}

fn ensure_lock_kind_name(kind: StandardLockKind) -> &'static str {
    match kind {
        StandardLockKind::Index => "index.lock",
        StandardLockKind::Head => "HEAD.lock",
    }
}

fn persist_publish(
    store: &GitOperationJournalStore,
    record: &mut GitJournalRecord,
    phase: GitJournalPhase,
) -> Result<(), String> {
    if record.phase != phase {
        transition(store, record, |next| next.phase = phase)?;
    }
    Ok(())
}

fn finalize(
    app_data_dir: &Path,
    store: &GitOperationJournalStore,
    record: &mut GitJournalRecord,
) -> Result<(), String> {
    let receipt = receipt(record)?;
    transition(store, record, |next| {
        next.receipt = Some(receipt);
        next.phase = GitJournalPhase::CompletedAwaitingAck;
        next.diagnostic = None;
        next.cleanup_complete = false;
    })?;
    complete_cleanup(app_data_dir, store, record)
}

pub(in crate::code_workspace::git_write::transaction) fn complete_cleanup(
    app_data_dir: &Path,
    store: &GitOperationJournalStore,
    record: &mut GitJournalRecord,
) -> Result<(), String> {
    if record.phase != GitJournalPhase::CompletedAwaitingAck {
        return Err("Git cleanup completion requires a completed receipt".to_string());
    }
    if record.cleanup_complete {
        return Ok(());
    }
    cleanup_after_publish(app_data_dir, record)?;
    transition(store, record, |next| next.cleanup_complete = true)
}

fn receipt(record: &GitJournalRecord) -> Result<CodeGitMutationReceipt, String> {
    match record.operation {
        CodeGitOperation::Stage | CodeGitOperation::Unstage => {
            Ok(CodeGitMutationReceipt::Index(CodeGitIndexMutationReceipt {
                operation_id: record.operation_id.clone(),
                operation: record.operation,
                scope: record.key.scope.clone(),
                thread_id: record.key.thread_id.clone(),
                request_generation: record.request.write_generation,
                before_snapshot_id: record.request.snapshot_id.clone(),
                file_id: record
                    .request
                    .file_id
                    .clone()
                    .ok_or_else(|| "Index recovery receipt lost its file id".to_string())?,
                disposition: if record.operation == CodeGitOperation::Stage {
                    "staged"
                } else {
                    "unstaged"
                }
                .to_string(),
            }))
        }
        CodeGitOperation::Commit => {
            let transaction = record
                .commit_transaction
                .as_ref()
                .ok_or_else(|| "Commit recovery receipt lost transaction evidence".to_string())?;
            Ok(CodeGitMutationReceipt::Commit(CodeGitCommitReceipt {
                operation_id: record.operation_id.clone(),
                operation: CodeGitOperation::Commit,
                scope: record.key.scope.clone(),
                thread_id: record.key.thread_id.clone(),
                request_generation: record.request.write_generation,
                before_snapshot_id: record.request.snapshot_id.clone(),
                previous_head: record.repository.previous_head.clone(),
                commit: transaction
                    .commit_oid
                    .clone()
                    .ok_or_else(|| "Commit recovery receipt lost its commit id".to_string())?,
                tree: transaction
                    .tree_oid
                    .clone()
                    .ok_or_else(|| "Commit recovery receipt lost its tree id".to_string())?,
                disposition: "committed".to_string(),
            }))
        }
    }
}

fn transition<F>(
    store: &GitOperationJournalStore,
    record: &mut GitJournalRecord,
    mutate: F,
) -> Result<(), String>
where
    F: FnOnce(&mut GitJournalRecord),
{
    let previous = record.clone();
    mutate(record);
    let mut journal = store.load()?;
    let current = journal
        .records
        .iter_mut()
        .find(|current| current.record_id == record.record_id)
        .ok_or_else(|| "Git recovery record disappeared during transition".to_string())?;
    if *current != previous {
        return Err("Git recovery record changed concurrently".to_string());
    }
    *current = record.clone();
    store.save(&journal)
}

fn require_publishable_evidence(
    record: &GitJournalRecord,
    minimum: GitJournalPhase,
) -> Result<(), String> {
    if record.phase != minimum
        || record
            .artifacts
            .index_artifact
            .as_ref()
            .is_none_or(|artifact| artifact.link_count != 2)
        || record
            .artifacts
            .head_artifact
            .as_ref()
            .is_none_or(|artifact| artifact.link_count != 2)
    {
        return Err("Live publish state lacks durable two-link artifact evidence".to_string());
    }
    Ok(())
}

fn is_durable_publish(record: &GitJournalRecord) -> bool {
    matches!(
        (record.operation, record.phase),
        (
            CodeGitOperation::Stage | CodeGitOperation::Unstage,
            GitJournalPhase::IndexPublished
        ) | (CodeGitOperation::Commit, GitJournalPhase::HeadPublished)
    )
}

fn validate_binding(binding: &CodeThreadBinding, record: &GitJournalRecord) -> Result<(), String> {
    if binding.execution_mode != CodeExecutionMode::Worktree
        || binding.community_id != record.key.scope.community_id
        || binding.project_dtag != record.key.scope.project_dtag
        || binding.repository_identity != record.key.scope.repository_identity
        || binding.codex_thread_id != record.key.thread_id
        || binding.execution_root != record.repository.root.exact_path
    {
        return Err("Git recovery binding does not match the durable record".to_string());
    }
    Ok(())
}

fn private_identity(
    evidence: &GitJournalArtifactEvidence,
    expected_label: &str,
) -> Result<FileIdentity, String> {
    let suffix = evidence
        .name
        .strip_prefix(expected_label)
        .and_then(|value| value.strip_prefix('-'))
        .ok_or_else(|| "Recovery artifact has the wrong private namespace".to_string())?;
    if suffix.len() != 32
        || !suffix
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("Recovery artifact has an invalid private random suffix".to_string());
    }
    let path = Path::new(&evidence.parent_path).join(&evidence.name);
    Ok(FileIdentity {
        path: path
            .to_str()
            .ok_or_else(|| "Recovery artifact path is not UTF-8".to_string())?
            .to_string(),
        device: evidence.device,
        inode: evidence.inode,
        owner: evidence.owner,
        mode: evidence.mode,
        link_count: evidence.link_count,
        size: evidence.size,
        digest: evidence.sha256.clone(),
    })
}

fn private_file_evidence(value: &FileIdentity) -> Result<GitJournalArtifactEvidence, String> {
    let path = Path::new(&value.path);
    let parent_path = path
        .parent()
        .and_then(Path::to_str)
        .ok_or_else(|| "Recovery candidate parent is not UTF-8".to_string())?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "Recovery candidate name is not UTF-8".to_string())?;
    Ok(GitJournalArtifactEvidence {
        parent_path: parent_path.to_string(),
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

fn owned_artifact(evidence: &GitJournalArtifactEvidence) -> OwnedArtifact {
    OwnedArtifact {
        component: evidence.name.clone(),
        identity: owned_identity(evidence),
    }
}

fn owned_lock(evidence: &GitJournalArtifactEvidence, kind: StandardLockKind) -> OwnedStandardLock {
    OwnedStandardLock {
        artifact_component: evidence.name.clone(),
        kind,
        identity: owned_identity(evidence),
    }
}

fn owned_identity(evidence: &GitJournalArtifactEvidence) -> OwnedFileIdentity {
    OwnedFileIdentity {
        device: evidence.device,
        inode: evidence.inode,
        owner: evidence.owner,
        mode: evidence.mode,
        link_count: evidence.link_count,
        size: evidence.size,
        sha256: evidence.sha256.clone(),
    }
}

fn journal_artifact(parent: &str, artifact: &OwnedArtifact) -> GitJournalArtifactEvidence {
    journal_owned(parent, &artifact.component, &artifact.identity)
}

fn journal_lock(parent: &str, lock: &OwnedStandardLock) -> GitJournalArtifactEvidence {
    journal_owned(parent, &lock.artifact_component, &lock.identity)
}

fn journal_owned(
    parent: &str,
    name: &str,
    identity: &OwnedFileIdentity,
) -> GitJournalArtifactEvidence {
    GitJournalArtifactEvidence {
        parent_path: parent.to_string(),
        name: name.to_string(),
        device: identity.device,
        inode: identity.inode,
        owner: identity.owner,
        mode: identity.mode,
        link_count: identity.link_count,
        size: identity.size,
        sha256: identity.sha256.clone(),
    }
}

fn format_git_timestamp(timestamp: super::super::journal::GitJournalTimestamp) -> String {
    let offset = i32::from(timestamp.offset_minutes);
    let sign = if offset < 0 { '-' } else { '+' };
    let absolute = offset.unsigned_abs();
    format!(
        "{} {sign}{:02}{:02}",
        timestamp.unix_seconds,
        absolute / 60,
        absolute % 60
    )
}

fn sanitize_diagnostic(value: &str) -> String {
    let value = value.trim();
    let value = if value.is_empty() {
        "Native Git recovery evidence became uncertain"
    } else {
        value
    };
    let mut end = value.len().min(MAX_DIAGNOSTIC_BYTES);
    while !value.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    value[..end].trim().to_string()
}
