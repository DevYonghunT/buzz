#![cfg(unix)]

use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::super::protocol::{CodeGitCommitReceipt, CodeGitIndexMutationReceipt};
use super::*;

fn hex_id(value: u64) -> String {
    format!("{value:064x}")
}

fn path_text(path: &Path) -> String {
    path.to_str()
        .unwrap_or_else(|| panic!("test path must be UTF-8: {}", path.display()))
        .to_string()
}

fn current_uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

fn path_identity(path: &Path, inode: u64, directory: bool) -> GitJournalPathIdentity {
    GitJournalPathIdentity {
        exact_path: path_text(path),
        device: 7,
        inode,
        owner: current_uid(),
        mode: if directory {
            u32::from(libc::S_IFDIR) | 0o700
        } else {
            u32::from(libc::S_IFREG) | 0o755
        },
        link_count: 1,
    }
}

fn executable_identity(path: &Path, inode: u64) -> GitJournalFileIdentity {
    GitJournalFileIdentity {
        exact_path: path_text(path),
        device: 7,
        inode,
        owner: current_uid(),
        mode: u32::from(libc::S_IFREG) | 0o755,
        link_count: 1,
        size: 1024,
        sha256: digest_bytes(b"git executable"),
    }
}

fn regular_file_identity(path: &Path, inode: u64, bytes: &[u8]) -> GitJournalFileIdentity {
    GitJournalFileIdentity {
        exact_path: path_text(path),
        device: 7,
        inode,
        owner: current_uid(),
        mode: u32::from(libc::S_IFREG) | 0o644,
        link_count: 1,
        size: bytes.len() as u64,
        sha256: digest_bytes(bytes),
    }
}

fn artifact(
    parent: &Path,
    name: impl Into<String>,
    inode: u64,
    link_count: u64,
    size: u64,
    sha256: String,
) -> GitJournalArtifactEvidence {
    GitJournalArtifactEvidence {
        parent_path: path_text(parent),
        name: name.into(),
        device: 7,
        inode,
        owner: current_uid(),
        mode: u32::from(libc::S_IFREG) | 0o600,
        link_count,
        size,
        sha256,
    }
}

fn prepared_record(base: &Path, seed: u64) -> GitJournalRecord {
    let repository_identity = digest_bytes(b"repository");
    let root = base.join("repository");
    let common = base.join("common");
    let admin = common.join("worktrees").join("admin");
    let candidate_parent = base.join("candidates");
    let previous_head = "1".repeat(40);
    let expected_index_digest = digest_bytes(format!("candidate-{seed}").as_bytes());
    let thread_id = format!("thread-{seed}");
    let operation_id = hex_id(seed.saturating_mul(2).saturating_add(2));

    GitJournalRecord {
        record_id: hex_id(seed.saturating_mul(2).saturating_add(1)),
        operation_id,
        key: GitJournalBindingKey {
            scope: CodeThreadBindingScope {
                community_id: "community".to_string(),
                project_dtag: "project".to_string(),
                repository_identity: repository_identity.clone(),
            },
            thread_id,
        },
        operation: CodeGitOperation::Stage,
        phase: GitJournalPhase::Prepared,
        request: GitJournalRequestCoordinate {
            write_generation: seed,
            snapshot_id: digest_bytes(format!("snapshot-{seed}").as_bytes()),
            file_id: Some(digest_bytes(format!("file-{seed}").as_bytes())),
        },
        input_digest: digest_bytes(format!("input-{seed}").as_bytes()),
        repository: GitJournalRepositoryEvidence {
            repository_identity,
            root: path_identity(&root, 10, true),
            admin: path_identity(&admin, 11, true),
            common_dir: path_identity(&common, 12, true),
            object_database: path_identity(&common.join("objects"), 13, true),
            worktree_git_file: regular_file_identity(
                &root.join(".git"),
                14,
                format!("gitdir: {}\n", admin.display()).as_bytes(),
            ),
            git_executable: executable_identity(Path::new("/usr/bin/git"), 15),
            previous_head: previous_head.clone(),
            head_file_digest: digest_bytes(format!("{previous_head}\n").as_bytes()),
            before_index_digest: digest_bytes(format!("before-index-{seed}").as_bytes()),
            before_config_digest: digest_bytes(format!("config-{seed}").as_bytes()),
        },
        artifacts: GitJournalArtifactSet {
            candidate_index: artifact(
                &candidate_parent,
                format!("candidate-{seed}.index"),
                20 + seed,
                1,
                128,
                expected_index_digest.clone(),
            ),
            source: Some(artifact(
                &candidate_parent,
                format!("source-{seed}.bin"),
                30 + seed,
                1,
                5,
                digest_bytes(format!("source-{seed}").as_bytes()),
            )),
            message: None,
            index_artifact_name: format!("schoolx-index-{seed}"),
            head_artifact_name: format!("schoolx-head-{seed}"),
            index_artifact: None,
            head_artifact: None,
        },
        index_transaction: Some(GitJournalIndexTransaction {
            expected_index_digest,
            expected_semantic_digest: digest_bytes(format!("expected-semantics-{seed}").as_bytes()),
            selected_path: "tracked.txt".to_string(),
            selected_mode: "100644".to_string(),
            selected_blob_oid: "2".repeat(40),
        }),
        commit_transaction: None,
        receipt: None,
        acknowledgement: None,
        diagnostic: None,
        recovery_started: false,
        cleanup_complete: false,
    }
}

fn acknowledged_record(base: &Path, seed: u64) -> GitJournalRecord {
    let mut record = prepared_record(base, seed);
    let transaction = record
        .index_transaction
        .as_ref()
        .unwrap_or_else(|| panic!("prepared index fixture must have transaction"));
    record.phase = GitJournalPhase::Acknowledged;
    record.cleanup_complete = true;
    record.artifacts.index_artifact = Some(artifact(
        Path::new(&record.repository.admin.exact_path),
        record.artifacts.index_artifact_name.clone(),
        100 + seed,
        2,
        record.artifacts.candidate_index.size,
        transaction.expected_index_digest.clone(),
    ));
    record.artifacts.head_artifact = Some(artifact(
        Path::new(&record.repository.admin.exact_path),
        record.artifacts.head_artifact_name.clone(),
        200 + seed,
        2,
        (record.repository.previous_head.len() + 1) as u64,
        digest_bytes(format!("{}\n", record.repository.previous_head).as_bytes()),
    ));
    let receipt = CodeGitMutationReceipt::Index(CodeGitIndexMutationReceipt {
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
            .unwrap_or_else(|| panic!("prepared index fixture must have file id")),
        disposition: "staged".to_string(),
    });
    record.acknowledgement = Some(GitJournalAckCoordinate {
        operation_id: record.operation_id.clone(),
        write_generation: record.request.write_generation + 1,
        snapshot_id: digest_bytes(format!("after-snapshot-{seed}").as_bytes()),
        receipt_digest: receipt_digest(&receipt)
            .unwrap_or_else(|error| panic!("receipt digest fixture failed: {error}")),
    });
    record.receipt = Some(receipt);
    record
}

fn prepared_commit_record(base: &Path, seed: u64) -> GitJournalRecord {
    let mut record = prepared_record(base, seed);
    let message_digest = digest_bytes(format!("message-{seed}").as_bytes());
    record.operation = CodeGitOperation::Commit;
    record.request.file_id = None;
    record.artifacts.candidate_index.sha256 = record.repository.before_index_digest.clone();
    record.artifacts.source = None;
    record.artifacts.message = Some(artifact(
        Path::new(&record.artifacts.candidate_index.parent_path),
        format!("message-{seed}.txt"),
        300 + seed,
        1,
        10,
        message_digest.clone(),
    ));
    record.index_transaction = None;
    record.commit_transaction = Some(GitJournalCommitTransaction {
        identity: CodeGitCommitIdentity {
            name: "SchoolX User".to_string(),
            email: "schoolx@example.com".to_string(),
        },
        author_timestamp: GitJournalTimestamp {
            unix_seconds: 1_700_000_000,
            offset_minutes: 540,
        },
        committer_timestamp: GitJournalTimestamp {
            unix_seconds: 1_700_000_000,
            offset_minutes: 540,
        },
        index_semantic_digest: digest_bytes(format!("commit-index-semantics-{seed}").as_bytes()),
        message_digest,
        tree_oid: None,
        commit_oid: None,
    });
    record
}

fn add_commit_admin_artifacts(record: &mut GitJournalRecord, link_count: u64) {
    let transaction = record
        .commit_transaction
        .as_ref()
        .unwrap_or_else(|| panic!("commit fixture must have transaction"));
    let commit_oid = transaction
        .commit_oid
        .as_ref()
        .unwrap_or_else(|| panic!("commit fixture must have commit oid"));
    record.artifacts.index_artifact = Some(artifact(
        Path::new(&record.repository.admin.exact_path),
        record.artifacts.index_artifact_name.clone(),
        400 + record.request.write_generation,
        link_count,
        record.artifacts.candidate_index.size,
        record.artifacts.candidate_index.sha256.clone(),
    ));
    record.artifacts.head_artifact = Some(artifact(
        Path::new(&record.repository.admin.exact_path),
        record.artifacts.head_artifact_name.clone(),
        500 + record.request.write_generation,
        link_count,
        (commit_oid.len() + 1) as u64,
        digest_bytes(format!("{commit_oid}\n").as_bytes()),
    ));
}

fn test_store() -> (TempDir, PathBuf, GitOperationJournalStore) {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
    let app_data = temp.path().join("app-data");
    fs::create_dir(&app_data).unwrap_or_else(|error| panic!("app-data create failed: {error}"));
    let app_data = app_data
        .canonicalize()
        .unwrap_or_else(|error| panic!("app-data canonicalize failed: {error}"));
    let store = GitOperationJournalStore::for_app_data(&app_data)
        .unwrap_or_else(|error| panic!("store create failed: {error}"));
    (temp, app_data, store)
}

fn create_code_directory(app_data: &Path, mode: u32) -> PathBuf {
    let code = app_data.join(JOURNAL_DIRECTORY);
    let mut builder = fs::DirBuilder::new();
    builder.mode(mode);
    builder
        .create(&code)
        .unwrap_or_else(|error| panic!("code directory create failed: {error}"));
    fs::set_permissions(&code, fs::Permissions::from_mode(mode))
        .unwrap_or_else(|error| panic!("code directory chmod failed: {error}"));
    code
}

fn write_file(path: &Path, bytes: &[u8], mode: u32) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .unwrap_or_else(|error| panic!("test file create failed: {error}"));
    file.write_all(bytes)
        .unwrap_or_else(|error| panic!("test file write failed: {error}"));
    file.sync_all()
        .unwrap_or_else(|error| panic!("test file sync failed: {error}"));
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .unwrap_or_else(|error| panic!("test file chmod failed: {error}"));
}

fn valid_payload(base: &Path) -> Vec<u8> {
    serde_json::to_vec_pretty(&GitOperationJournal {
        version: GIT_OPERATION_JOURNAL_VERSION,
        records: vec![prepared_record(base, 1)],
    })
    .unwrap_or_else(|error| panic!("fixture encode failed: {error}"))
}

#[test]
fn absent_load_is_zero_mutation() {
    let (_temp, app_data, store) = test_store();
    let before =
        fs::metadata(&app_data).unwrap_or_else(|error| panic!("app-data metadata failed: {error}"));

    let loaded = store
        .load()
        .unwrap_or_else(|error| panic!("absent load failed: {error}"));

    assert_eq!(loaded, GitOperationJournal::default());
    assert!(!app_data.join(JOURNAL_DIRECTORY).exists());
    let after =
        fs::metadata(&app_data).unwrap_or_else(|error| panic!("app-data metadata failed: {error}"));
    assert_eq!(before.permissions().mode(), after.permissions().mode());
}

#[test]
fn absent_file_in_existing_private_directory_is_zero_mutation() {
    let (_temp, app_data, store) = test_store();
    let code = create_code_directory(&app_data, 0o700);
    let marker = code.join("foreign-marker");
    write_file(&marker, b"preserve", 0o600);

    let loaded = store
        .load()
        .unwrap_or_else(|error| panic!("absent file load failed: {error}"));

    assert_eq!(loaded, GitOperationJournal::default());
    assert!(!store.path().exists());
    assert_eq!(
        fs::read(&marker).unwrap_or_else(|error| panic!("marker read failed: {error}")),
        b"preserve"
    );
    assert_eq!(
        fs::read_dir(code)
            .unwrap_or_else(|error| panic!("code read_dir failed: {error}"))
            .count(),
        1
    );
}

#[test]
fn private_store_roundtrips() {
    let (_temp, app_data, store) = test_store();
    let journal = GitOperationJournal {
        version: GIT_OPERATION_JOURNAL_VERSION,
        records: vec![prepared_record(&app_data, 1)],
    };

    store
        .save(&journal)
        .unwrap_or_else(|error| panic!("journal save failed: {error}"));

    assert_eq!(
        store
            .load()
            .unwrap_or_else(|error| panic!("journal load failed: {error}")),
        journal
    );
    assert_eq!(
        fs::metadata(app_data.join(JOURNAL_DIRECTORY))
            .unwrap_or_else(|error| panic!("code metadata failed: {error}"))
            .permissions()
            .mode()
            & 0o7777,
        0o700
    );
    assert_eq!(
        fs::metadata(store.path())
            .unwrap_or_else(|error| panic!("journal metadata failed: {error}"))
            .permissions()
            .mode()
            & 0o7777,
        0o600
    );
}

#[test]
fn wrong_directory_and_file_modes_are_preserved() {
    let (_temp, app_data, store) = test_store();
    let code = create_code_directory(&app_data, 0o755);
    assert!(store.load().is_err());
    assert_eq!(
        fs::metadata(&code)
            .unwrap_or_else(|error| panic!("code metadata failed: {error}"))
            .permissions()
            .mode()
            & 0o7777,
        0o755
    );

    let (_temp, app_data, store) = test_store();
    let code = create_code_directory(&app_data, 0o700);
    let bytes = valid_payload(&app_data);
    write_file(&code.join(JOURNAL_FILE), &bytes, 0o644);
    assert!(store.load().is_err());
    assert_eq!(
        fs::read(store.path()).unwrap_or_else(|error| panic!("journal read failed: {error}")),
        bytes
    );
    assert_eq!(
        fs::metadata(store.path())
            .unwrap_or_else(|error| panic!("journal metadata failed: {error}"))
            .permissions()
            .mode()
            & 0o7777,
        0o644
    );
}

#[test]
fn symlink_journal_is_rejected_without_touching_target() {
    let (temp, app_data, store) = test_store();
    let code = create_code_directory(&app_data, 0o700);
    let target = temp.path().join("foreign-journal.json");
    let bytes = valid_payload(&app_data);
    write_file(&target, &bytes, 0o600);
    std::os::unix::fs::symlink(&target, code.join(JOURNAL_FILE))
        .unwrap_or_else(|error| panic!("symlink create failed: {error}"));

    assert!(store.load().is_err());
    assert_eq!(
        fs::read(&target).unwrap_or_else(|error| panic!("target read failed: {error}")),
        bytes
    );
    assert!(fs::symlink_metadata(store.path())
        .unwrap_or_else(|error| panic!("symlink metadata failed: {error}"))
        .file_type()
        .is_symlink());
}

#[test]
fn oversized_and_malformed_bytes_are_preserved() {
    let (_temp, app_data, store) = test_store();
    let code = create_code_directory(&app_data, 0o700);
    let oversized = vec![b'x'; MAX_GIT_OPERATION_JOURNAL_BYTES as usize + 1];
    write_file(&code.join(JOURNAL_FILE), &oversized, 0o600);
    assert!(store.load().is_err());
    assert_eq!(
        fs::metadata(store.path())
            .unwrap_or_else(|error| panic!("journal metadata failed: {error}"))
            .len(),
        oversized.len() as u64
    );

    let (_temp, app_data, store) = test_store();
    let code = create_code_directory(&app_data, 0o700);
    let malformed = b"{this is not JSON";
    write_file(&code.join(JOURNAL_FILE), malformed, 0o600);
    assert!(store.load().is_err());
    assert_eq!(
        fs::read(store.path()).unwrap_or_else(|error| panic!("journal read failed: {error}")),
        malformed
    );
}

#[test]
fn duplicate_blocker_and_unknown_field_bytes_are_preserved() {
    let (_temp, app_data, store) = test_store();
    let code = create_code_directory(&app_data, 0o700);
    let first = prepared_record(&app_data, 1);
    let mut second = prepared_record(&app_data, 2);
    second.key = first.key.clone();
    let duplicate = serde_json::to_vec_pretty(&GitOperationJournal {
        version: GIT_OPERATION_JOURNAL_VERSION,
        records: vec![first, second],
    })
    .unwrap_or_else(|error| panic!("duplicate fixture encode failed: {error}"));
    write_file(&code.join(JOURNAL_FILE), &duplicate, 0o600);
    assert!(store.load().is_err());
    assert_eq!(
        fs::read(store.path()).unwrap_or_else(|error| panic!("journal read failed: {error}")),
        duplicate
    );

    let (_temp, app_data, store) = test_store();
    let code = create_code_directory(&app_data, 0o700);
    let mut unknown = serde_json::to_value(GitOperationJournal::default())
        .unwrap_or_else(|error| panic!("unknown fixture encode failed: {error}"));
    unknown
        .as_object_mut()
        .unwrap_or_else(|| panic!("journal fixture must be an object"))
        .insert("unknown".to_string(), serde_json::Value::Bool(true));
    let unknown = serde_json::to_vec_pretty(&unknown)
        .unwrap_or_else(|error| panic!("unknown fixture encode failed: {error}"));
    write_file(&code.join(JOURNAL_FILE), &unknown, 0o600);
    assert!(store.load().is_err());
    assert_eq!(
        fs::read(store.path()).unwrap_or_else(|error| panic!("journal read failed: {error}")),
        unknown
    );
}

#[test]
fn acknowledged_tombstones_are_bounded_to_newest_history() {
    let (_temp, app_data, store) = test_store();
    let records = (0..300)
        .map(|seed| acknowledged_record(&app_data, seed))
        .collect();
    let journal = GitOperationJournal {
        version: GIT_OPERATION_JOURNAL_VERSION,
        records,
    };

    store
        .save(&journal)
        .unwrap_or_else(|error| panic!("journal save failed: {error}"));
    let loaded = store
        .load()
        .unwrap_or_else(|error| panic!("journal load failed: {error}"));

    assert_eq!(loaded.records.len(), MAX_ACKNOWLEDGED_TOMBSTONES);
    assert_eq!(
        loaded.records[0].record_id,
        acknowledged_record(&app_data, 44).record_id
    );
    assert_eq!(
        loaded.records[MAX_ACKNOWLEDGED_TOMBSTONES - 1].record_id,
        acknowledged_record(&app_data, 299).record_id
    );
    assert!(loaded
        .records
        .iter()
        .all(|record| record.phase == GitJournalPhase::Acknowledged));
}

#[test]
fn index_phase_validation_preserves_frozen_source_and_planned_names() {
    let (_temp, app_data, _store) = test_store();
    let mut stage = prepared_record(&app_data, 1);
    stage.phase = GitJournalPhase::ObjectWritten;
    assert!(stage.validate().is_ok());

    let mut escaped_source = stage.clone();
    escaped_source
        .artifacts
        .source
        .as_mut()
        .unwrap_or_else(|| panic!("stage fixture must have source"))
        .parent_path = escaped_source.repository.root.exact_path.clone();
    assert!(escaped_source.validate().is_err());

    let mut unstage = stage.clone();
    unstage.operation = CodeGitOperation::Unstage;
    unstage.artifacts.source = None;
    assert!(unstage.validate().is_err());

    let mut deletion = stage.clone();
    deletion.artifacts.source = None;
    let transaction = deletion
        .index_transaction
        .as_mut()
        .unwrap_or_else(|| panic!("stage fixture must have transaction"));
    transaction.selected_mode = "000000".to_string();
    transaction.selected_blob_oid = "0".repeat(40);
    assert!(deletion.validate().is_err());

    let transaction = stage
        .index_transaction
        .as_ref()
        .unwrap_or_else(|| panic!("stage fixture must have transaction"));
    stage.phase = GitJournalPhase::ArtifactsReady;
    stage.artifacts.index_artifact = Some(artifact(
        Path::new(&stage.repository.admin.exact_path),
        stage.artifacts.index_artifact_name.clone(),
        600,
        1,
        stage.artifacts.candidate_index.size,
        transaction.expected_index_digest.clone(),
    ));
    stage.artifacts.head_artifact = Some(artifact(
        Path::new(&stage.repository.admin.exact_path),
        stage.artifacts.head_artifact_name.clone(),
        601,
        1,
        (stage.repository.previous_head.len() + 1) as u64,
        stage.repository.head_file_digest.clone(),
    ));
    assert!(stage.validate().is_ok());

    let mut wrong_planned_name = stage;
    wrong_planned_name.artifacts.index_artifact_name = "different-index-artifact".to_string();
    assert!(wrong_planned_name.validate().is_err());
}

#[test]
fn commit_phases_require_objects_artifacts_receipt_and_exact_ack() {
    let (_temp, app_data, _store) = test_store();
    let mut record = prepared_commit_record(&app_data, 2);
    assert!(record.validate().is_ok());

    record.phase = GitJournalPhase::ObjectWritten;
    assert!(record.validate().is_err());
    let transaction = record
        .commit_transaction
        .as_mut()
        .unwrap_or_else(|| panic!("commit fixture must have transaction"));
    transaction.tree_oid = Some("3".repeat(40));
    transaction.commit_oid = Some("4".repeat(40));
    assert!(record.validate().is_ok());

    record.phase = GitJournalPhase::ArtifactsReady;
    assert!(record.validate().is_err());
    add_commit_admin_artifacts(&mut record, 1);
    assert!(record.validate().is_ok());

    record.phase = GitJournalPhase::LocksReady;
    record
        .artifacts
        .index_artifact
        .as_mut()
        .unwrap_or_else(|| panic!("commit fixture must have index artifact"))
        .link_count = 2;
    record
        .artifacts
        .head_artifact
        .as_mut()
        .unwrap_or_else(|| panic!("commit fixture must have HEAD artifact"))
        .link_count = 2;
    assert!(record.validate().is_ok());

    record.phase = GitJournalPhase::CompletedAwaitingAck;
    assert!(record.validate().is_err());
    let transaction = record
        .commit_transaction
        .as_ref()
        .unwrap_or_else(|| panic!("commit fixture must have transaction"));
    let receipt = CodeGitMutationReceipt::Commit(CodeGitCommitReceipt {
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
            .unwrap_or_else(|| panic!("commit fixture must have commit oid")),
        tree: transaction
            .tree_oid
            .clone()
            .unwrap_or_else(|| panic!("commit fixture must have tree oid")),
        disposition: "committed".to_string(),
    });
    record.receipt = Some(receipt.clone());
    assert!(record.validate().is_ok());

    record.phase = GitJournalPhase::Acknowledged;
    assert!(record.validate().is_err());
    record.acknowledgement = Some(GitJournalAckCoordinate {
        operation_id: record.operation_id.clone(),
        write_generation: record.request.write_generation + 1,
        snapshot_id: digest_bytes(b"commit after snapshot"),
        receipt_digest: receipt_digest(&receipt)
            .unwrap_or_else(|error| panic!("receipt digest fixture failed: {error}")),
    });
    record.cleanup_complete = true;
    assert!(record.validate().is_ok());

    record
        .acknowledgement
        .as_mut()
        .unwrap_or_else(|| panic!("commit fixture must have acknowledgement"))
        .snapshot_id = record.request.snapshot_id.clone();
    assert!(record.validate().is_err());
}

#[test]
fn sealed_commit_transition_rejects_impossible_phase_combinations() {
    let (_temp, app_data, _store) = test_store();
    let mut record = prepared_commit_record(&app_data, 8);
    let sealed_digest = digest_bytes(b"sealed post-write-tree candidate");
    record.artifacts.candidate_index.sha256 = sealed_digest;
    record.artifacts.candidate_index.inode += 1;
    record
        .commit_transaction
        .as_mut()
        .unwrap_or_else(|| panic!("commit fixture must have transaction"))
        .tree_oid = Some("3".repeat(40));
    assert!(record.validate().is_ok());

    let mut impossible_commit = record.clone();
    impossible_commit
        .commit_transaction
        .as_mut()
        .unwrap_or_else(|| panic!("commit fixture must have transaction"))
        .commit_oid = Some("4".repeat(40));
    assert!(impossible_commit.validate().is_err());

    let mut impossible_artifacts = record;
    impossible_artifacts
        .commit_transaction
        .as_mut()
        .unwrap_or_else(|| panic!("commit fixture must have transaction"))
        .commit_oid = Some("4".repeat(40));
    add_commit_admin_artifacts(&mut impossible_artifacts, 1);
    assert!(impossible_artifacts.validate().is_err());
}

#[test]
fn completed_record_rejects_generation_that_cannot_advance() {
    let (_temp, app_data, _store) = test_store();
    let mut record = acknowledged_record(&app_data, 9);
    record.phase = GitJournalPhase::CompletedAwaitingAck;
    record.request.write_generation = u64::MAX;
    record.acknowledgement = None;
    assert!(record.validate().is_err());
}

#[test]
fn repository_authority_paths_are_exactly_bound() {
    let (_temp, app_data, _store) = test_store();
    let mut record = prepared_record(&app_data, 10);
    record.repository.object_database.exact_path =
        app_data.join("other-objects").display().to_string();
    assert!(record.validate().is_err());

    let mut record = prepared_record(&app_data, 11);
    record.repository.worktree_git_file.exact_path =
        app_data.join("other-git").display().to_string();
    assert!(record.validate().is_err());

    let mut record = prepared_record(&app_data, 12);
    record.repository.admin.exact_path = app_data
        .join("foreign")
        .join("worktrees")
        .join("admin")
        .display()
        .to_string();
    assert!(record.validate().is_err());
}

#[test]
fn schema_rejects_nul_keys_duplicate_ids_and_record_overflow() {
    let (_temp, app_data, _store) = test_store();
    let mut nul = prepared_record(&app_data, 1);
    nul.key.thread_id = "thread\0other".to_string();
    assert!(nul.validate().is_err());

    let first = acknowledged_record(&app_data, 1);
    let mut second = acknowledged_record(&app_data, 2);
    second.operation_id = first.operation_id.clone();
    assert!(GitOperationJournal {
        version: GIT_OPERATION_JOURNAL_VERSION,
        records: vec![first.clone(), second],
    }
    .validate()
    .is_err());

    assert!(GitOperationJournal {
        version: GIT_OPERATION_JOURNAL_VERSION,
        records: vec![first; MAX_GIT_OPERATION_JOURNAL_RECORDS + 1],
    }
    .validate()
    .is_err());
}

#[test]
fn save_refuses_to_overwrite_malformed_existing_bytes() {
    let (_temp, app_data, store) = test_store();
    let code = create_code_directory(&app_data, 0o700);
    let malformed = b"not-json";
    write_file(&code.join(JOURNAL_FILE), malformed, 0o600);
    let valid = GitOperationJournal {
        version: GIT_OPERATION_JOURNAL_VERSION,
        records: vec![prepared_record(&app_data, 1)],
    };

    assert!(store.save(&valid).is_err());
    assert_eq!(
        fs::read(store.path()).unwrap_or_else(|error| panic!("journal read failed: {error}")),
        malformed
    );
    let temps = fs::read_dir(&code)
        .unwrap_or_else(|error| panic!("code read_dir failed: {error}"))
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
        .count();
    assert_eq!(temps, 0);
}

#[test]
fn existing_hard_linked_journal_is_rejected_and_preserved() {
    let (temp, app_data, store) = test_store();
    let code = create_code_directory(&app_data, 0o700);
    let bytes = valid_payload(&app_data);
    write_file(&code.join(JOURNAL_FILE), &bytes, 0o600);
    let alias = temp.path().join("journal-alias.json");
    fs::hard_link(store.path(), &alias)
        .unwrap_or_else(|error| panic!("hard link create failed: {error}"));

    assert!(store.load().is_err());
    assert_eq!(
        fs::read(store.path()).unwrap_or_else(|error| panic!("journal read failed: {error}")),
        bytes
    );
    assert_eq!(
        fs::read(alias).unwrap_or_else(|error| panic!("alias read failed: {error}")),
        bytes
    );
}

#[test]
fn wrong_mode_save_does_not_repair_existing_file() {
    let (_temp, app_data, store) = test_store();
    let code = create_code_directory(&app_data, 0o700);
    let bytes = valid_payload(&app_data);
    write_file(&code.join(JOURNAL_FILE), &bytes, 0o644);
    let journal = GitOperationJournal {
        version: GIT_OPERATION_JOURNAL_VERSION,
        records: vec![prepared_record(&app_data, 2)],
    };

    assert!(store.save(&journal).is_err());
    assert_eq!(
        fs::read(store.path()).unwrap_or_else(|error| panic!("journal read failed: {error}")),
        bytes
    );
    assert_eq!(
        fs::metadata(store.path())
            .unwrap_or_else(|error| panic!("journal metadata failed: {error}"))
            .permissions()
            .mode()
            & 0o7777,
        0o644
    );
}

#[test]
fn load_rejects_unknown_nested_record_field() {
    let (_temp, app_data, store) = test_store();
    let code = create_code_directory(&app_data, 0o700);
    let mut value = serde_json::to_value(GitOperationJournal {
        version: GIT_OPERATION_JOURNAL_VERSION,
        records: vec![prepared_record(&app_data, 1)],
    })
    .unwrap_or_else(|error| panic!("fixture encode failed: {error}"));
    value["records"][0]
        .as_object_mut()
        .unwrap_or_else(|| panic!("record fixture must be an object"))
        .insert("foreignEvidence".to_string(), serde_json::Value::Null);
    let bytes = serde_json::to_vec_pretty(&value)
        .unwrap_or_else(|error| panic!("fixture encode failed: {error}"));
    write_file(&code.join(JOURNAL_FILE), &bytes, 0o600);

    assert!(store.load().is_err());
    assert_eq!(
        fs::read(store.path()).unwrap_or_else(|error| panic!("journal read failed: {error}")),
        bytes
    );
}

#[test]
fn opening_existing_file_never_follows_fifo_or_directory() {
    let (_temp, app_data, store) = test_store();
    let code = create_code_directory(&app_data, 0o700);
    fs::create_dir(code.join(JOURNAL_FILE))
        .unwrap_or_else(|error| panic!("journal directory fixture failed: {error}"));

    assert!(store.load().is_err());
    assert!(store.path().is_dir());
}

#[test]
fn private_file_fixture_uses_regular_files() {
    let temp = tempfile::tempdir().unwrap_or_else(|error| panic!("tempdir failed: {error}"));
    let path = temp.path().join("file");
    write_file(&path, b"content", 0o600);
    let file = File::open(&path).unwrap_or_else(|error| panic!("file open failed: {error}"));
    assert!(file
        .metadata()
        .unwrap_or_else(|error| panic!("file metadata failed: {error}"))
        .is_file());
}
