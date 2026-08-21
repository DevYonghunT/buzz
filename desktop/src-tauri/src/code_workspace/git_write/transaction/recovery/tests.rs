#![cfg(unix)]

use std::fs;
use std::os::unix::fs::DirBuilderExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::super::super::journal::{
    GitJournalArtifactSet, GitJournalBindingKey, GitJournalCommitTransaction,
    GitJournalIndexTransaction, GitJournalRequestCoordinate, GitJournalTimestamp,
    GitOperationJournal, GIT_OPERATION_JOURNAL_VERSION,
};
use super::super::super::owned_lock::{LockAcquisition, StandardLockKind};
use super::super::super::private_artifact::PrivateArtifactStore;
use super::super::super::repository::{
    inspect_repository, prepare_commit_inputs, prepare_index_candidate,
};
use super::super::{private_evidence, repository_evidence};
use super::*;
use crate::code_workspace::git_write::protocol::CodeGitCommitIdentity;
use crate::code_workspace::{CodeExecutionMode, CodeThreadBinding, CodeThreadBindingScope};

struct Fixture {
    _repository: tempfile::TempDir,
    _worktree_parent: tempfile::TempDir,
    app_data: tempfile::TempDir,
    root: PathBuf,
    binding: CodeThreadBinding,
}

impl Fixture {
    fn prepared_stage() -> Result<(Self, GitJournalRecord), String> {
        let repository = tempfile::tempdir().map_err(|error| error.to_string())?;
        run(repository.path(), &["init", "-q"])?;
        run(
            repository.path(),
            &["config", "--local", "user.name", "Recovery Test"],
        )?;
        run(
            repository.path(),
            &["config", "--local", "user.email", "recovery@example.com"],
        )?;
        fs::write(repository.path().join("tracked.txt"), b"base\n")
            .map_err(|error| error.to_string())?;
        run(repository.path(), &["add", "tracked.txt"])?;
        run(repository.path(), &["commit", "-q", "-m", "base"])?;

        let worktree_parent = tempfile::tempdir().map_err(|error| error.to_string())?;
        let root = worktree_parent.path().join("managed");
        let root_text = root
            .to_str()
            .ok_or_else(|| "Recovery test worktree path is not UTF-8".to_string())?;
        run(
            repository.path(),
            &["worktree", "add", "-q", "--detach", root_text, "HEAD"],
        )?;
        let root = root.canonicalize().map_err(|error| error.to_string())?;
        fs::write(root.join("tracked.txt"), b"candidate\n").map_err(|error| error.to_string())?;

        let common = resolve_git_path(&root, "--git-common-dir")?;
        let scope = CodeThreadBindingScope {
            community_id: "recovery-community".to_string(),
            project_dtag: "recovery-project".to_string(),
            repository_identity: crate::code_workspace::repository_identity(&common)?,
        };
        let binding = CodeThreadBinding {
            community_id: scope.community_id.clone(),
            project_dtag: scope.project_dtag.clone(),
            repository_identity: scope.repository_identity.clone(),
            codex_thread_id: "recovery-thread".to_string(),
            execution_mode: CodeExecutionMode::Worktree,
            execution_root: root.to_string_lossy().to_string(),
            base_ref: run(&root, &["rev-parse", "HEAD"])?.trim().to_string(),
            worktree_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
        };

        let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
        let mut code = fs::DirBuilder::new();
        code.mode(0o700);
        code.create(app_data.path().join("code"))
            .map_err(|error| error.to_string())?;
        let projection = inspect_repository(&binding)?;
        let private = PrivateArtifactStore::for_mutation(app_data.path())?;
        let prepared = prepare_index_candidate(&private, &projection, "tracked.txt", true)?;
        let admin = PinnedAdminDirectory::pin(&projection.admin)?;
        let index_name = admin.issue_artifact_component()?;
        let head_name = admin.issue_artifact_component()?;
        let record = GitJournalRecord {
            record_id: "1".repeat(64),
            operation_id: "2".repeat(64),
            key: GitJournalBindingKey {
                scope,
                thread_id: binding.codex_thread_id.clone(),
            },
            operation: CodeGitOperation::Stage,
            phase: GitJournalPhase::Prepared,
            request: GitJournalRequestCoordinate {
                write_generation: 7,
                snapshot_id: "3".repeat(64),
                file_id: Some("4".repeat(64)),
            },
            input_digest: "5".repeat(64),
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
                expected_index_digest: prepared.expected_index_digest,
                expected_semantic_digest: prepared.expected_semantic_digest,
                selected_path: "tracked.txt".to_string(),
                selected_mode: prepared.selected_mode,
                selected_blob_oid: prepared.selected_blob_oid,
            }),
            commit_transaction: None,
            receipt: None,
            acknowledgement: None,
            diagnostic: None,
            recovery_started: false,
            cleanup_complete: false,
        };
        let fixture = Self {
            _repository: repository,
            _worktree_parent: worktree_parent,
            app_data,
            root,
            binding,
        };
        fixture.save(&record)?;
        Ok((fixture, record))
    }

    fn prepared_commit() -> Result<(Self, GitJournalRecord), String> {
        let (fixture, stage) = Self::prepared_stage()?;
        let private = PrivateArtifactStore::for_mutation(fixture.app_data.path())?;
        if let Some(source) = stage.artifacts.source.as_ref() {
            private.remove(&private_identity(source, "stage-source")?)?;
        }
        private.remove(&private_identity(
            &stage.artifacts.candidate_index,
            "candidate-index",
        )?)?;
        run(&fixture.root, &["add", "tracked.txt"])?;
        let projection = inspect_repository(&fixture.binding)?;
        let prepared = prepare_commit_inputs(&private, &projection, "Recovered commit")?;
        let admin = PinnedAdminDirectory::pin(&projection.admin)?;
        let index_name = admin.issue_artifact_component()?;
        let head_name = admin.issue_artifact_component()?;
        let message_digest = prepared.message.digest.clone();
        let record = GitJournalRecord {
            record_id: "6".repeat(64),
            operation_id: "7".repeat(64),
            key: stage.key,
            operation: CodeGitOperation::Commit,
            phase: GitJournalPhase::Prepared,
            request: GitJournalRequestCoordinate {
                write_generation: 8,
                snapshot_id: "8".repeat(64),
                file_id: None,
            },
            input_digest: "9".repeat(64),
            repository: repository_evidence(&projection),
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
                identity: CodeGitCommitIdentity {
                    name: "Recovery Test".to_string(),
                    email: "recovery@example.com".to_string(),
                },
                author_timestamp: GitJournalTimestamp {
                    unix_seconds: 1_700_000_000,
                    offset_minutes: 0,
                },
                committer_timestamp: GitJournalTimestamp {
                    unix_seconds: 1_700_000_000,
                    offset_minutes: 0,
                },
                index_semantic_digest: projection.index_semantic_digest,
                message_digest,
                tree_oid: None,
                commit_oid: None,
            }),
            receipt: None,
            acknowledgement: None,
            diagnostic: None,
            recovery_started: false,
            cleanup_complete: false,
        };
        fixture.save(&record)?;
        Ok((fixture, record))
    }

    fn store(&self) -> Result<GitOperationJournalStore, String> {
        GitOperationJournalStore::for_app_data(self.app_data.path())
    }

    fn save(&self, record: &GitJournalRecord) -> Result<(), String> {
        self.store()?.save(&GitOperationJournal {
            version: GIT_OPERATION_JOURNAL_VERSION,
            records: vec![record.clone()],
        })
    }

    fn load(&self) -> Result<GitJournalRecord, String> {
        self.store()?
            .load()?
            .records
            .into_iter()
            .next()
            .ok_or_else(|| "Recovery test journal is empty".to_string())
    }

    fn recover(&self, record: &GitJournalRecord) -> Result<(), String> {
        recover_record(self.app_data.path(), &self.binding, &record.record_id)
    }

    fn admin(&self, record: &GitJournalRecord) -> PathBuf {
        PathBuf::from(&record.repository.admin.exact_path)
    }
}

#[test]
fn before_state_resume_converges_to_completed_awaiting_ack() -> Result<(), String> {
    let (fixture, prepared) = Fixture::prepared_stage()?;
    let expected_receipt = receipt(&prepared)?;

    fixture.recover(&prepared)?;

    let recovered = fixture.load()?;
    assert_eq!(recovered.phase, GitJournalPhase::CompletedAwaitingAck);
    assert!(recovered.cleanup_complete);
    assert_eq!(recovered.receipt, Some(expected_receipt));
    assert_eq!(
        run(&fixture.root, &["diff", "--cached", "--name-only"])?.trim(),
        "tracked.txt"
    );
    Ok(())
}

#[test]
fn expected_after_state_converges_with_the_same_receipt() -> Result<(), String> {
    let (fixture, mut record) = Fixture::prepared_stage()?;
    let expected_receipt = receipt(&record)?;
    install_locked_artifacts(&fixture, &mut record)?;
    let admin = fixture.admin(&record);
    fs::rename(admin.join("index.lock"), admin.join("index")).map_err(|error| error.to_string())?;

    fixture.recover(&record)?;

    let recovered = fixture.load()?;
    assert_eq!(recovered.phase, GitJournalPhase::CompletedAwaitingAck);
    assert!(recovered.cleanup_complete);
    assert_eq!(recovered.receipt, Some(expected_receipt));
    assert!(!admin.join("HEAD.lock").exists());
    assert!(!admin.join(&record.artifacts.index_artifact_name).exists());
    assert!(!admin.join(&record.artifacts.head_artifact_name).exists());
    Ok(())
}

#[test]
fn completed_receipt_reenters_partial_cleanup_before_becoming_ackable() -> Result<(), String> {
    let (fixture, mut record) = Fixture::prepared_stage()?;
    install_locked_artifacts(&fixture, &mut record)?;
    let admin_path = fixture.admin(&record);
    fs::rename(admin_path.join("index.lock"), admin_path.join("index"))
        .map_err(|error| error.to_string())?;
    record.phase = GitJournalPhase::CompletedAwaitingAck;
    record.receipt = Some(receipt(&record)?);
    record.cleanup_complete = false;
    fixture.save(&record)?;

    let private = PrivateArtifactStore::for_mutation(fixture.app_data.path())?;
    let source = private_identity(
        record
            .artifacts
            .source
            .as_ref()
            .ok_or_else(|| "stage cleanup fixture lost source".to_string())?,
        "stage-source",
    )?;
    private.remove(&source)?;

    fixture.recover(&record)?;

    let recovered = fixture.load()?;
    assert_eq!(recovered.phase, GitJournalPhase::CompletedAwaitingAck);
    assert!(recovered.cleanup_complete);
    assert!(!admin_path.join("HEAD.lock").exists());
    assert!(!admin_path
        .join(&record.artifacts.index_artifact_name)
        .exists());
    assert!(!admin_path
        .join(&record.artifacts.head_artifact_name)
        .exists());
    assert!(!Path::new(&record.artifacts.candidate_index.parent_path)
        .join(&record.artifacts.candidate_index.name)
        .exists());
    Ok(())
}

#[test]
fn commit_write_tree_candidate_transition_is_adopted_and_completed() -> Result<(), String> {
    let (fixture, record) = Fixture::prepared_commit()?;
    let repository = RecoveryRepository::open(&fixture.binding, &record)?;
    let private = PrivateRecovery::open_exact(fixture.app_data.path(), &record)?;
    let before = private.candidate.clone();
    repository.runner.run(GitWriteCommand::WriteTree {
        index: private.candidate.clone(),
    })?;
    let after = private
        .store
        .secure_after_mutation(Path::new(&private.candidate.path))?;
    assert_ne!(after, before, "write-tree fixture did not mutate its index");

    fixture.recover(&record)?;

    let recovered = fixture.load()?;
    assert_eq!(recovered.phase, GitJournalPhase::CompletedAwaitingAck);
    assert!(recovered.cleanup_complete);
    let receipt = recovered
        .receipt
        .as_ref()
        .ok_or_else(|| "commit recovery lost its receipt".to_string())?;
    let CodeGitMutationReceipt::Commit(commit) = receipt else {
        return Err("commit recovery returned an index receipt".to_string());
    };
    assert_eq!(
        run(&fixture.root, &["rev-parse", "HEAD"])?.trim(),
        commit.commit
    );
    Ok(())
}

#[test]
fn commit_post_rename_state_converges_with_same_receipt() -> Result<(), String> {
    let (fixture, mut record) = Fixture::prepared_commit()?;
    let repository = RecoveryRepository::open(&fixture.binding, &record)?;
    let mut private = PrivateRecovery::open_commit(fixture.app_data.path(), &record, &repository)?;
    let store = fixture.store()?;
    ensure_commit_objects(&store, &mut record, &repository, &mut private)?;
    install_locked_artifacts(&fixture, &mut record)?;
    let expected_receipt = receipt(&record)?;
    let admin = fixture.admin(&record);
    fs::rename(admin.join("HEAD.lock"), admin.join("HEAD")).map_err(|error| error.to_string())?;

    fixture.recover(&record)?;

    let recovered = fixture.load()?;
    assert_eq!(recovered.phase, GitJournalPhase::CompletedAwaitingAck);
    assert!(recovered.cleanup_complete);
    assert_eq!(recovered.receipt, Some(expected_receipt));
    assert!(!admin.join("index.lock").exists());
    assert!(!admin.join(&record.artifacts.index_artifact_name).exists());
    assert!(!admin.join(&record.artifacts.head_artifact_name).exists());
    Ok(())
}

#[test]
fn third_live_state_becomes_sticky_uncertain() -> Result<(), String> {
    let (fixture, record) = Fixture::prepared_stage()?;
    fs::write(fixture.root.join("tracked.txt"), b"third state\n")
        .map_err(|error| error.to_string())?;
    run(&fixture.root, &["add", "tracked.txt"])?;

    fixture.recover(&record)?;
    let uncertain = fixture.load()?;
    assert_eq!(uncertain.phase, GitJournalPhase::Uncertain);
    assert!(uncertain.recovery_started);
    assert!(uncertain
        .diagnostic
        .as_deref()
        .is_some_and(|message| message.contains("third state")));
    let before_retry = fs::read(fixture.store()?.path()).map_err(|error| error.to_string())?;

    fixture.recover(&record)?;

    let after_retry = fs::read(fixture.store()?.path()).map_err(|error| error.to_string())?;
    assert_eq!(after_retry, before_retry);
    assert_eq!(fixture.load()?, uncertain);
    Ok(())
}

#[test]
fn foreign_standard_locks_are_preserved() -> Result<(), String> {
    let (fixture, record) = Fixture::prepared_stage()?;
    let admin = fixture.admin(&record);
    let foreign_index = b"foreign index lock\n";
    let foreign_head = b"foreign HEAD lock\n";
    fs::write(admin.join("index.lock"), foreign_index).map_err(|error| error.to_string())?;
    fs::write(admin.join("HEAD.lock"), foreign_head).map_err(|error| error.to_string())?;

    fixture.recover(&record)?;

    assert_eq!(fixture.load()?.phase, GitJournalPhase::Uncertain);
    assert_eq!(
        fs::read(admin.join("index.lock")).map_err(|error| error.to_string())?,
        foreign_index
    );
    assert_eq!(
        fs::read(admin.join("HEAD.lock")).map_err(|error| error.to_string())?,
        foreign_head
    );
    Ok(())
}

fn install_locked_artifacts(
    fixture: &Fixture,
    record: &mut GitJournalRecord,
) -> Result<(), String> {
    let private = PrivateArtifactStore::for_mutation(fixture.app_data.path())?;
    let candidate = private_identity(
        &record.artifacts.candidate_index,
        match record.operation {
            CodeGitOperation::Stage | CodeGitOperation::Unstage => "candidate-index",
            CodeGitOperation::Commit => "commit-index",
        },
    )?;
    let candidate = private.read(&candidate)?;
    let admin = PinnedAdminDirectory::pin(Path::new(&record.repository.admin.exact_path))?;
    let index_name = IssuedArtifactComponent::from_journal(&record.artifacts.index_artifact_name)?;
    let head_name = IssuedArtifactComponent::from_journal(&record.artifacts.head_artifact_name)?;
    let index = admin.create_artifact(&index_name, &candidate)?;
    let head_value = match record.operation {
        CodeGitOperation::Stage | CodeGitOperation::Unstage => {
            record.repository.previous_head.as_str()
        }
        CodeGitOperation::Commit => record
            .commit_transaction
            .as_ref()
            .and_then(|transaction| transaction.commit_oid.as_deref())
            .ok_or_else(|| "commit lock fixture lost its commit object".to_string())?,
    };
    let head_bytes = format!("{head_value}\n");
    let head = admin.create_artifact(&head_name, head_bytes.as_bytes())?;
    let index = acquired(admin.acquire_standard_lock(&index, StandardLockKind::Index)?)?;
    let head = acquired(admin.acquire_standard_lock(&head, StandardLockKind::Head)?)?;
    record.artifacts.index_artifact =
        Some(journal_lock(&record.repository.admin.exact_path, &index));
    record.artifacts.head_artifact = Some(journal_lock(&record.repository.admin.exact_path, &head));
    record.phase = GitJournalPhase::LocksReady;
    fixture.save(record)
}

fn acquired(value: LockAcquisition) -> Result<OwnedStandardLock, String> {
    match value {
        LockAcquisition::Acquired(lock) => Ok(lock),
        other => Err(format!(
            "expected newly acquired recovery lock, got {other:?}"
        )),
    }
}

fn resolve_git_path(root: &Path, argument: &str) -> Result<PathBuf, String> {
    let value = run(root, &["rev-parse", argument])?;
    let value = PathBuf::from(value.trim());
    let path = if value.is_absolute() {
        value
    } else {
        root.join(value)
    };
    path.canonicalize().map_err(|error| error.to_string())
}

fn run(root: &Path, arguments: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(arguments)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}
