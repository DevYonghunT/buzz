//! Subprocess crash/reopen coverage for durable Git write transactions.

mod fixture;

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::{Command, Output};

use fixture::{CrashFixture, CrashOperation, CrashRequest};

use super::super::journal::GitJournalPhase;
use super::fault::{
    TransactionFaultBoundary as Boundary, INJECTED_CRASH_EXIT_CODE, TEST_FAULT_BOUNDARY_ENV,
};

const CRASH_REQUEST_ENV: &str = "SCHOOLX_CODE_GIT_WRITE_CRASH_REQUEST_V1";
const CRASH_REQUEST_VERSION: u32 = 1;
const MAX_CRASH_REQUEST_BYTES: usize = 64 * 1024;

const STAGE_BOUNDARIES: &[Boundary] = &[
    Boundary::PreparedPersisted,
    Boundary::BlobObjectWritten,
    Boundary::ObjectWrittenPersisted,
    Boundary::IndexArtifactDurable,
    Boundary::HeadArtifactDurable,
    Boundary::ArtifactsReadyPersisted,
    Boundary::IndexLockDurable,
    Boundary::HeadLockDurable,
    Boundary::LocksReadyPersisted,
    Boundary::BeforeIndexPublish,
    Boundary::IndexPublishDurable,
    Boundary::IndexPublishedPersisted,
    Boundary::CompletedReceiptPersisted,
    Boundary::CleanupCompleted,
    Boundary::ResponseReady,
];

const COMMIT_BOUNDARIES: &[Boundary] = &[
    Boundary::PreparedPersisted,
    Boundary::TreeObjectWritten,
    Boundary::TreeEvidencePersisted,
    Boundary::CommitObjectWritten,
    Boundary::ObjectWrittenPersisted,
    Boundary::IndexArtifactDurable,
    Boundary::HeadArtifactDurable,
    Boundary::ArtifactsReadyPersisted,
    Boundary::IndexLockDurable,
    Boundary::HeadLockDurable,
    Boundary::LocksReadyPersisted,
    Boundary::BeforeHeadPublish,
    Boundary::HeadPublishDurable,
    Boundary::HeadPublishedPersisted,
    Boundary::CompletedReceiptPersisted,
    Boundary::CleanupCompleted,
    Boundary::ResponseReady,
];

#[test]
fn stage_subprocess_crash_reopen_matrix_is_exact_once() -> Result<(), String> {
    run_crash_matrix(CrashOperation::Stage, STAGE_BOUNDARIES)
}

#[test]
fn commit_subprocess_crash_reopen_matrix_is_exact_once() -> Result<(), String> {
    run_crash_matrix(CrashOperation::Commit, COMMIT_BOUNDARIES)
}

#[test]
fn unstage_subprocess_crash_reopen_matches_index_protocol() -> Result<(), String> {
    run_crash_matrix(
        CrashOperation::Unstage,
        &[
            Boundary::PreparedPersisted,
            Boundary::HeadLockDurable,
            Boundary::IndexPublishDurable,
            Boundary::CompletedReceiptPersisted,
        ],
    )
}

fn run_crash_matrix(operation: CrashOperation, boundaries: &[Boundary]) -> Result<(), String> {
    for boundary in boundaries {
        let result = (|| -> Result<(), String> {
            let fixture = CrashFixture::prepare(operation)?;
            spawn_crash_child(&fixture.request(operation, *boundary)?)?;
            let interrupted = fixture.load_record()?;
            if interrupted.operation != operation.journal_operation() {
                return Err("crash journal recorded the wrong operation".to_string());
            }
            assert_interrupted_phase(operation, *boundary, &interrupted)?;
            fixture.recover_startup()?;
            let completed = fixture.load_record()?;
            fixture.assert_completed(operation, &completed)?;
            let expected = completed
                .receipt
                .clone()
                .ok_or_else(|| "recovered crash lost its receipt".to_string())?;
            if fixture.exact_retry(operation, &completed)? != expected {
                return Err("exact retry returned a different receipt".to_string());
            }
            fixture.recover_startup()?;
            if fixture.load_record()? != completed {
                return Err("second startup recovery changed completed evidence".to_string());
            }
            Ok(())
        })();
        result.map_err(|error| {
            format!("{operation:?} crash/reopen failed at {boundary:?}: {error}")
        })?;
    }
    Ok(())
}

fn assert_interrupted_phase(
    operation: CrashOperation,
    boundary: Boundary,
    record: &super::super::journal::GitJournalRecord,
) -> Result<(), String> {
    let pre_artifact = if operation == CrashOperation::Unstage {
        GitJournalPhase::Prepared
    } else {
        GitJournalPhase::ObjectWritten
    };
    let expected = match boundary {
        Boundary::PreparedPersisted
        | Boundary::BlobObjectWritten
        | Boundary::TreeObjectWritten
        | Boundary::TreeEvidencePersisted
        | Boundary::CommitObjectWritten => GitJournalPhase::Prepared,
        Boundary::ObjectWrittenPersisted => GitJournalPhase::ObjectWritten,
        Boundary::IndexArtifactDurable | Boundary::HeadArtifactDurable => pre_artifact,
        Boundary::ArtifactsReadyPersisted
        | Boundary::IndexLockDurable
        | Boundary::HeadLockDurable => GitJournalPhase::ArtifactsReady,
        Boundary::LocksReadyPersisted
        | Boundary::BeforeIndexPublish
        | Boundary::IndexPublishDurable
        | Boundary::BeforeHeadPublish
        | Boundary::HeadPublishDurable => GitJournalPhase::LocksReady,
        Boundary::IndexPublishedPersisted => GitJournalPhase::IndexPublished,
        Boundary::HeadPublishedPersisted => GitJournalPhase::HeadPublished,
        Boundary::CompletedReceiptPersisted
        | Boundary::CleanupCompleted
        | Boundary::ResponseReady => GitJournalPhase::CompletedAwaitingAck,
        Boundary::AcknowledgementPersisted => GitJournalPhase::Acknowledged,
    };
    if record.phase != expected {
        return Err(format!(
            "checkpoint {boundary:?} persisted {:?}, expected {expected:?}",
            record.phase
        ));
    }
    if boundary == Boundary::CompletedReceiptPersisted
        && (record.receipt.is_none() || record.cleanup_complete)
    {
        return Err("completed receipt checkpoint did not precede cleanup".to_string());
    }
    if matches!(
        boundary,
        Boundary::CleanupCompleted | Boundary::ResponseReady
    ) && (!record.cleanup_complete || record.receipt.is_none())
    {
        return Err("response checkpoint did not follow durable cleanup".to_string());
    }
    if operation == CrashOperation::Commit {
        let commit = record
            .commit_transaction
            .as_ref()
            .ok_or_else(|| "commit checkpoint lost transaction evidence".to_string())?;
        if boundary == Boundary::TreeObjectWritten && commit.tree_oid.is_some() {
            return Err("tree object checkpoint unexpectedly persisted tree evidence".to_string());
        }
        if boundary == Boundary::TreeEvidencePersisted && commit.tree_oid.is_none() {
            return Err("tree evidence checkpoint did not persist the tree id".to_string());
        }
        if boundary == Boundary::CommitObjectWritten && commit.commit_oid.is_some() {
            return Err(
                "commit object checkpoint unexpectedly persisted commit evidence".to_string(),
            );
        }
    }
    Ok(())
}

#[test]
fn durable_index_publish_ignores_later_external_index_drift() -> Result<(), String> {
    let fixture = CrashFixture::prepare(CrashOperation::Stage)?;
    spawn_crash_child(&fixture.request(CrashOperation::Stage, Boundary::IndexPublishedPersisted)?)?;
    let interrupted = fixture.load_record()?;
    if interrupted.phase != GitJournalPhase::IndexPublished {
        return Err("index drift fixture did not stop at durable publish".to_string());
    }
    fs::write(
        fixture.managed_root.join("tracked.txt"),
        b"external index\n",
    )
    .map_err(|error| error.to_string())?;
    fixture::run(&fixture.managed_root, &["add", "tracked.txt"])?;

    fixture.recover_startup()?;
    let completed = fixture.load_record()?;
    if fixture::run(&fixture.managed_root, &["show", ":tracked.txt"])? != "external index\n" {
        return Err("recovery overwrote post-publish index drift".to_string());
    }
    assert_same_retry(&fixture, CrashOperation::Stage, &completed)?;
    fixture.assert_preserved()
}

#[test]
fn durable_head_publish_ignores_later_external_head_drift() -> Result<(), String> {
    let fixture = CrashFixture::prepare(CrashOperation::Commit)?;
    spawn_crash_child(&fixture.request(CrashOperation::Commit, Boundary::HeadPublishedPersisted)?)?;
    let interrupted = fixture.load_record()?;
    if interrupted.phase != GitJournalPhase::HeadPublished {
        return Err("HEAD drift fixture did not stop at durable publish".to_string());
    }
    atomically_replace(
        &fixture.admin()?.join("HEAD"),
        format!("{}\n", fixture.base_head).as_bytes(),
    )?;

    fixture.recover_startup()?;
    let completed = fixture.load_record()?;
    if fixture::run(&fixture.managed_root, &["rev-parse", "HEAD"])?.trim() != fixture.base_head {
        return Err("recovery overwrote post-publish HEAD drift".to_string());
    }
    assert_same_retry(&fixture, CrashOperation::Commit, &completed)?;
    fixture.assert_preserved()
}

fn assert_same_retry(
    fixture: &CrashFixture,
    operation: CrashOperation,
    completed: &super::super::journal::GitJournalRecord,
) -> Result<(), String> {
    if completed.phase != GitJournalPhase::CompletedAwaitingAck || !completed.cleanup_complete {
        return Err("durable publish did not finalize and clean up".to_string());
    }
    let receipt = completed
        .receipt
        .clone()
        .ok_or_else(|| "durable publish lost its receipt".to_string())?;
    if fixture.exact_retry(operation, completed)? != receipt {
        return Err("durable publish retry returned a different receipt".to_string());
    }
    Ok(())
}

#[test]
fn acknowledgement_response_loss_retries_exact_tombstone() -> Result<(), String> {
    let fixture = CrashFixture::prepare(CrashOperation::AcknowledgeStage)?;
    spawn_crash_child(&fixture.request(
        CrashOperation::AcknowledgeStage,
        Boundary::AcknowledgementPersisted,
    )?)?;
    let acknowledged = fixture.load_record()?;
    if acknowledged.phase != GitJournalPhase::Acknowledged
        || acknowledged.acknowledgement.is_none()
        || !acknowledged.cleanup_complete
    {
        return Err("acknowledgement response loss was not durable".to_string());
    }
    let retry = fixture.retry_acknowledgement(&acknowledged)?;
    if retry.operation_id != acknowledged.operation_id || retry.disposition != "acknowledged" {
        return Err("acknowledgement retry returned the wrong tombstone".to_string());
    }
    if fixture::run(&fixture.managed_root, &["show", ":tracked.txt"])? != "staged version\n" {
        return Err("acknowledgement retry mutated the index".to_string());
    }
    fixture.assert_preserved()
}

#[test]
fn foreign_standard_locks_are_never_removed_by_failed_transactions() -> Result<(), String> {
    for (operation, name) in [
        (CrashOperation::Stage, "index.lock"),
        (CrashOperation::Commit, "HEAD.lock"),
    ] {
        let fixture = CrashFixture::prepare(operation)?;
        let path = fixture.admin()?.join(name);
        let bytes = format!("foreign {name}\n").into_bytes();
        write_foreign(&path, &bytes)?;
        let error = fixture::execute_request(&fixture.request(operation, Boundary::ResponseReady)?)
            .expect_err("foreign standard lock must reject the transaction");
        if !error.to_ascii_lowercase().contains("lock") {
            return Err(format!("foreign {name} returned the wrong error: {error}"));
        }
        if fs::read(&path).map_err(|error| error.to_string())? != bytes {
            return Err(format!("foreign {name} bytes were changed"));
        }
        if fixture::run(&fixture.managed_root, &["rev-parse", "HEAD"])?.trim() != fixture.base_head
        {
            return Err(format!("foreign {name} rejection changed HEAD"));
        }
        if fixture::run(&fixture.managed_root, &["show", ":tracked.txt"])?
            != if operation == CrashOperation::Commit {
                "staged version\n"
            } else {
                "base\n"
            }
        {
            return Err(format!("foreign {name} rejection changed the index"));
        }
        fixture.assert_preserved()?;
    }
    Ok(())
}

#[test]
fn crash_recovery_preserves_replaced_lock_and_planned_artifact() -> Result<(), String> {
    for (boundary, component) in [
        (Boundary::IndexArtifactDurable, "planned-index-artifact"),
        (Boundary::IndexLockDurable, "index.lock"),
        (Boundary::HeadLockDurable, "HEAD.lock"),
    ] {
        let fixture = CrashFixture::prepare(CrashOperation::Stage)?;
        spawn_crash_child(&fixture.request(CrashOperation::Stage, boundary)?)?;
        let interrupted = fixture.load_record()?;
        let path = if boundary == Boundary::IndexArtifactDurable {
            fixture
                .admin()?
                .join(&interrupted.artifacts.index_artifact_name)
        } else {
            fixture.admin()?.join(component)
        };
        fs::remove_file(&path).map_err(|error| error.to_string())?;
        let foreign = format!("foreign replacement at {boundary:?}\n").into_bytes();
        write_foreign(&path, &foreign)?;

        fixture.recover_startup()?;
        if fs::read(&path).map_err(|error| error.to_string())? != foreign {
            return Err(format!(
                "recovery removed foreign replacement at {boundary:?}"
            ));
        }
        if fixture.load_record()?.phase != GitJournalPhase::Uncertain {
            return Err(format!(
                "foreign replacement at {boundary:?} was not sticky uncertain"
            ));
        }
        if fixture::run(&fixture.managed_root, &["rev-parse", "HEAD"])?.trim() != fixture.base_head
        {
            return Err("foreign recovery changed HEAD".to_string());
        }
        fixture.assert_preserved()?;
    }
    Ok(())
}

fn spawn_crash_child(request: &CrashRequest) -> Result<(), String> {
    let output = spawn_child(request)?;
    if output.status.code() != Some(INJECTED_CRASH_EXIT_CODE) {
        return Err(format!(
            "crash child for {:?}/{:?} exited as {:?}; stdout={}; stderr={}",
            request.operation,
            request.boundary,
            output.status,
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }
    Ok(())
}

fn spawn_child(request: &CrashRequest) -> Result<Output, String> {
    let encoded = serde_json::to_string(request)
        .map_err(|error| format!("failed to encode Git crash request: {error}"))?;
    if encoded.is_empty() || encoded.len() > MAX_CRASH_REQUEST_BYTES {
        return Err(format!(
            "Git crash request must be 1..={MAX_CRASH_REQUEST_BYTES} bytes"
        ));
    }
    let boundary = serde_json::to_string(&request.boundary)
        .map_err(|error| format!("failed to encode Git crash boundary: {error}"))?;
    let executable = std::env::current_exe()
        .map_err(|error| format!("failed to resolve Git crash test executable: {error}"))?;
    Command::new(executable)
        .args([
            "--exact",
            "code_workspace::git_write::transaction::crash_tests::git_write_transaction_crash_subprocess_entry",
            "--ignored",
            "--nocapture",
        ])
        .env(CRASH_REQUEST_ENV, encoded)
        .env(TEST_FAULT_BOUNDARY_ENV, boundary)
        .output()
        .map_err(|error| format!("failed to start Git crash child: {error}"))
}

fn run_crash_child_from_env() -> Result<(), String> {
    let encoded = std::env::var(CRASH_REQUEST_ENV)
        .map_err(|_| "Git transaction crash request is missing".to_string())?;
    if encoded.is_empty() || encoded.len() > MAX_CRASH_REQUEST_BYTES {
        return Err(format!(
            "Git crash request must be 1..={MAX_CRASH_REQUEST_BYTES} bytes"
        ));
    }
    let request: CrashRequest = serde_json::from_str(&encoded)
        .map_err(|error| format!("Git transaction crash request is invalid: {error}"))?;
    if request.version != CRASH_REQUEST_VERSION {
        return Err(format!(
            "unsupported Git transaction crash request version {}",
            request.version
        ));
    }
    let boundary = std::env::var(TEST_FAULT_BOUNDARY_ENV)
        .map_err(|_| "Git transaction fault boundary is missing".to_string())?;
    let boundary: Boundary = serde_json::from_str(&boundary)
        .map_err(|error| format!("Git transaction fault boundary is invalid: {error}"))?;
    if boundary != request.boundary {
        return Err("Git crash request boundary does not match the fault hook".to_string());
    }
    fixture::execute_request(&request)
}

fn write_foreign(path: &Path, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|error| error.to_string())?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| error.to_string())?;
    fs::File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| error.to_string())?;
    sync_parent(path)
}

fn atomically_replace(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "replacement path has no parent".to_string())?;
    let temp = parent.join("schoolx-external-head-replacement");
    write_foreign(&temp, bytes)?;
    fs::rename(&temp, path).map_err(|error| error.to_string())?;
    sync_parent(path)
}

fn sync_parent(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "test path has no parent".to_string())?;
    fs::File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|error| error.to_string())
}

#[test]
#[ignore = "private Git transaction crash subprocess entry"]
fn git_write_transaction_crash_subprocess_entry() {
    if let Err(error) = run_crash_child_from_env() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
