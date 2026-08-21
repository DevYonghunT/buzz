use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::journal::{
    GitJournalAckCoordinate, GitJournalBindingKey, GitJournalPhase, GitJournalRecord,
    GitOperationJournal, GitOperationJournalStore,
};
use super::protocol::*;
use super::repository::*;
use super::transaction::{self, CommitClaim, IndexMutationClaim};
use crate::code_workspace::{CodeThreadBinding, CodeThreadBindingScope};

pub(super) const MAX_ACTION_FILES: usize = 250;
pub(super) const MAX_PATCH_BYTES: usize = 256 * 1024;
pub(super) const MAX_COMMIT_MESSAGE_BYTES: usize = 64 * 1024;

#[derive(Clone, Default)]
pub(crate) struct CodeGitWriteState {
    inner: Arc<Mutex<MemoryState>>,
    repository_write: Arc<Mutex<()>>,
}

#[derive(Default)]
struct MemoryState {
    bindings: HashMap<GitJournalBindingKey, BindingMemory>,
}

#[derive(Default)]
struct BindingMemory {
    write_generation: u64,
    status_revision: u64,
    snapshot_sequence: u64,
    projection_digest: String,
    snapshot: Option<Snapshot>,
}

#[derive(Clone)]
pub(super) struct Snapshot {
    pub(super) id: String,
    pub(super) write_generation: u64,
    pub(super) sequence: u64,
    pub(super) preimage: String,
    pub(super) head: String,
    pub(super) root: PathBuf,
    pub(super) admin: PathBuf,
    pub(super) identity: Option<CodeGitCommitIdentity>,
    pub(super) files: HashMap<String, SnapshotFile>,
}

#[derive(Clone)]
pub(super) struct SnapshotFile {
    pub(super) path: String,
    pub(super) staged: bool,
    pub(super) unstaged: bool,
}

#[derive(Clone)]
pub(crate) struct GitWriteContext {
    pub(crate) app_data_dir: PathBuf,
    pub(crate) binding: CodeThreadBinding,
    pub(crate) runtime_generation: u64,
    pub(crate) task: CodeGitChangeSet,
    pub(crate) activity_blocker: Option<String>,
}

pub(super) struct RepoProjection {
    pub(super) repository_identity: String,
    pub(super) root: PathBuf,
    pub(super) admin: PathBuf,
    pub(super) common: PathBuf,
    pub(super) head: String,
    pub(super) index_digest: String,
    pub(super) index_semantic_digest: String,
    pub(super) head_file_digest: String,
    pub(super) config_digest: String,
    pub(super) preimage: String,
    pub(super) identity: Option<CodeGitCommitIdentity>,
    pub(super) staged: Vec<RawChange>,
    pub(super) unstaged: Vec<RawChange>,
    pub(super) has_conflicts: bool,
    pub(super) root_identity: super::git_command::DirectoryIdentity,
    pub(super) admin_identity: super::git_command::DirectoryIdentity,
    pub(super) common_identity: super::git_command::DirectoryIdentity,
    pub(super) object_database_identity: super::git_command::DirectoryIdentity,
    pub(super) worktree_git_file: super::git_command::FileIdentity,
    pub(super) git_identity: super::git_command::FileIdentity,
    pub(super) index_file: super::git_command::FileIdentity,
    pub(super) head_file: super::git_command::FileIdentity,
    pub(super) runner: super::git_command::PinnedGitWriteRepository,
}

#[derive(Clone)]
pub(super) struct RawChange {
    pub(super) path: String,
    pub(super) status: CodeGitChangeStatus,
    pub(super) binary: bool,
    pub(super) additions: usize,
    pub(super) deletions: usize,
    pub(super) patch: String,
    pub(super) truncated: bool,
}

pub(crate) fn status(
    state: &CodeGitWriteState,
    input: CodeGitStatusInput,
    context: GitWriteContext,
) -> Result<CodeGitStatus, String> {
    validate_context(&input.scope, &input.thread_id, &context.binding)?;
    let key = binding_key(&input.scope, &input.thread_id);
    let journal = GitOperationJournalStore::for_app_data(&context.app_data_dir)?.load()?;
    if let Some(status) =
        recovery_required_from_journal(state, &input, context.runtime_generation, &key, &journal)?
    {
        return Ok(status);
    }

    super::with_git_authority(|| {
        let projection = match inspect_repository(&context.binding) {
            Ok(value) => value,
            Err(error) => {
                let mut memory = lock_memory(&state.inner)?;
                let binding = memory.bindings.entry(key.clone()).or_default();
                binding.write_generation = binding
                    .write_generation
                    .max(durable_write_generation(&journal, &key));
                binding.status_revision = binding.status_revision.saturating_add(1);
                return Ok(CodeGitStatus::Blocked {
                    runtime_generation: context.runtime_generation,
                    status_revision: binding.status_revision,
                    write_generation: binding.write_generation,
                    scope: input.scope,
                    thread_id: input.thread_id,
                    reason: error,
                    remediation: "Close active Git operations, restore a supported detached managed worktree, then refresh."
                        .to_string(),
                });
            }
        };

        let blocking_receipt = blocking_record(&journal, &key)
            .filter(|record| {
                record.phase == GitJournalPhase::CompletedAwaitingAck && record.cleanup_complete
            })
            .and_then(|record| record.receipt.clone());
        let projection_digest = digest_text(&format!(
            "{}\0{}\0{}\0{}\0{:?}",
            projection.preimage,
            context.activity_blocker.as_deref().unwrap_or_default(),
            blocking_receipt
                .as_ref()
                .map(CodeGitMutationReceipt::operation_id)
                .unwrap_or_default(),
            projection.has_conflicts,
            projection.identity
        ));

        let mut memory = lock_memory(&state.inner)?;
        let binding_memory = memory.bindings.entry(key).or_default();
        binding_memory.write_generation =
            binding_memory
                .write_generation
                .max(durable_write_generation(
                    &journal,
                    &binding_key(&input.scope, &input.thread_id),
                ));
        if binding_memory.projection_digest != projection_digest
            || binding_memory.snapshot.is_none()
        {
            binding_memory.status_revision = binding_memory.status_revision.saturating_add(1);
            binding_memory.snapshot_sequence = binding_memory.snapshot_sequence.saturating_add(1);
            binding_memory.projection_digest = projection_digest;
            binding_memory.snapshot = Some(issue_snapshot(
                binding_memory.write_generation,
                binding_memory.snapshot_sequence,
                &projection,
            )?);
        }
        let snapshot = binding_memory
            .snapshot
            .as_ref()
            .ok_or_else(|| "SchoolX Code Git snapshot was not issued".to_string())?;
        let staged = changes_with_ids(&projection.staged, snapshot)?;
        let unstaged = changes_with_ids(&projection.unstaged, snapshot)?;
        let blocker = context
            .activity_blocker
            .clone()
            .or_else(|| {
                projection
                    .has_conflicts
                    .then(|| "Resolve Git conflicts before writing.".to_string())
            })
            .or_else(|| {
                blocking_receipt.as_ref().map(|_| {
                    "A completed Git operation must be acknowledged after refresh.".to_string()
                })
            });
        let stage = capability(
            blocker.clone(),
            !unstaged.files.is_empty(),
            "There are no unstaged files.",
        );
        let unstage = capability(
            blocker.clone(),
            !staged.files.is_empty(),
            "There are no staged files.",
        );
        let commit_blocker = blocker.or_else(|| {
            projection.identity.is_none().then(|| {
                "Set repository-local user.name and user.email before committing.".to_string()
            })
        });
        let commit = capability(
            commit_blocker,
            !staged.files.is_empty(),
            "There are no staged files to commit.",
        );
        let task = task_with_snapshot_ids(&context.task, snapshot)?;
        Ok(CodeGitStatus::Ready {
            runtime_generation: context.runtime_generation,
            status_revision: binding_memory.status_revision,
            write_generation: binding_memory.write_generation,
            snapshot_sequence: snapshot.sequence,
            scope: input.scope,
            thread_id: input.thread_id,
            snapshot_id: snapshot.id.clone(),
            head_commit: projection.head,
            task: Box::new(task),
            staged: Box::new(staged),
            unstaged: Box::new(unstaged),
            has_conflicts: projection.has_conflicts,
            commit_identity: projection.identity,
            capabilities: Box::new(CodeGitCapabilities {
                stage,
                unstage,
                commit,
            }),
            blocking_receipt: blocking_receipt.map(Box::new),
        })
    })
}

/// Return a durable recovery blocker without touching the managed worktree.
/// Command facades use this before root/task-diff reads so a damaged or
/// partially published repository can still reach explicit reconciliation.
pub(crate) fn recovery_required_status(
    state: &CodeGitWriteState,
    app_data_dir: &Path,
    input: &CodeGitStatusInput,
    runtime_generation: u64,
) -> Result<Option<CodeGitStatus>, String> {
    let key = binding_key(&input.scope, &input.thread_id);
    let journal = GitOperationJournalStore::for_app_data(app_data_dir)?.load()?;
    recovery_required_from_journal(state, input, runtime_generation, &key, &journal)
}

fn recovery_required_from_journal(
    state: &CodeGitWriteState,
    input: &CodeGitStatusInput,
    runtime_generation: u64,
    key: &GitJournalBindingKey,
    journal: &GitOperationJournal,
) -> Result<Option<CodeGitStatus>, String> {
    let Some(record) = blocking_record(journal, key) else {
        return Ok(None);
    };
    if record.phase == GitJournalPhase::CompletedAwaitingAck && record.cleanup_complete {
        return Ok(None);
    }
    let mut memory = lock_memory(&state.inner)?;
    let binding = memory.bindings.entry(key.clone()).or_default();
    binding.write_generation = binding
        .write_generation
        .max(durable_write_generation(journal, key));
    binding.status_revision = binding.status_revision.saturating_add(1);
    Ok(Some(CodeGitStatus::RecoveryRequired {
        runtime_generation,
        status_revision: binding.status_revision,
        write_generation: binding.write_generation,
        scope: input.scope.clone(),
        thread_id: input.thread_id.clone(),
        operation: CodeGitRecoveryOperation {
            operation_id: record.operation_id.clone(),
            operation: record.operation,
            journal_state: journal_state(record).to_string(),
        },
    }))
}

pub(crate) fn blocked_status(
    state: &CodeGitWriteState,
    input: CodeGitStatusInput,
    runtime_generation: u64,
    reason: String,
) -> Result<CodeGitStatus, String> {
    let key = binding_key(&input.scope, &input.thread_id);
    let mut memory = lock_memory(&state.inner)?;
    let binding = memory.bindings.entry(key).or_default();
    binding.status_revision = binding.status_revision.saturating_add(1);
    Ok(CodeGitStatus::Blocked {
        runtime_generation,
        status_revision: binding.status_revision,
        write_generation: binding.write_generation,
        scope: input.scope,
        thread_id: input.thread_id,
        reason,
        remediation: "Use an active managed worktree with detached HEAD, then refresh.".to_string(),
    })
}

pub(crate) fn stage(
    state: &CodeGitWriteState,
    app_data_dir: &Path,
    binding: &CodeThreadBinding,
    input: CodeGitIndexMutationInput,
) -> Result<CodeGitIndexMutationReceipt, String> {
    mutate_index(state, app_data_dir, binding, input, true)
}

pub(crate) fn unstage(
    state: &CodeGitWriteState,
    app_data_dir: &Path,
    binding: &CodeThreadBinding,
    input: CodeGitIndexMutationInput,
) -> Result<CodeGitIndexMutationReceipt, String> {
    mutate_index(state, app_data_dir, binding, input, false)
}

fn mutate_index(
    state: &CodeGitWriteState,
    app_data_dir: &Path,
    binding: &CodeThreadBinding,
    input: CodeGitIndexMutationInput,
    is_stage: bool,
) -> Result<CodeGitIndexMutationReceipt, String> {
    validate_context(&input.scope, &input.thread_id, binding)?;
    let key = binding_key(&input.scope, &input.thread_id);
    let operation = if is_stage {
        CodeGitOperation::Stage
    } else {
        CodeGitOperation::Unstage
    };
    let input_digest = digest_json(&(operation, &input))?;
    let _repository_guard = state
        .repository_write
        .lock()
        .map_err(|_| "SchoolX Code Git write lock is unavailable".to_string())?;
    super::with_git_authority(|| {
        if let Some(receipt) = exact_retry_receipt(app_data_dir, &key, operation, &input_digest)? {
            return match receipt {
                CodeGitMutationReceipt::Index(value) => Ok(value),
                CodeGitMutationReceipt::Commit(_) => {
                    Err("Git mutation coordinate belongs to a different operation".to_string())
                }
            };
        }
        ensure_no_blocking_journal(app_data_dir, &key)?;
        let snapshot = require_snapshot(state, &key, input.write_generation, &input.snapshot_id)?;
        let file = snapshot
            .files
            .get(&input.file_id)
            .ok_or_else(|| "Git file coordinate is unknown or expired".to_string())?
            .clone();
        if (is_stage && !file.unstaged) || (!is_stage && !file.staged) {
            return Err(
                "Git file coordinate does not belong to the requested change lane".to_string(),
            );
        }
        let projection = inspect_repository(binding)?;
        ensure_snapshot_preimage(&snapshot, &projection)?;
        let receipt = transaction::execute_index(
            IndexMutationClaim {
                app_data_dir,
                binding,
                key: key.clone(),
                input_digest,
                write_generation: input.write_generation,
                snapshot_id: input.snapshot_id,
                file_id: input.file_id,
                path: file.path,
                operation,
            },
            projection,
        )?;
        advance_write_generation(state, &key)?;
        Ok(receipt)
    })
}

pub(crate) fn commit(
    state: &CodeGitWriteState,
    app_data_dir: &Path,
    binding: &CodeThreadBinding,
    input: CodeGitCommitInput,
) -> Result<CodeGitCommitReceipt, String> {
    validate_context(&input.scope, &input.thread_id, binding)?;
    let key = binding_key(&input.scope, &input.thread_id);
    let input_digest = digest_json(&(CodeGitOperation::Commit, &input))?;
    let _repository_guard = state
        .repository_write
        .lock()
        .map_err(|_| "SchoolX Code Git write lock is unavailable".to_string())?;
    super::with_git_authority(|| {
        if let Some(receipt) =
            exact_retry_receipt(app_data_dir, &key, CodeGitOperation::Commit, &input_digest)?
        {
            return match receipt {
                CodeGitMutationReceipt::Commit(value) => Ok(value),
                CodeGitMutationReceipt::Index(_) => {
                    Err("Git mutation coordinate belongs to a different operation".to_string())
                }
            };
        }
        ensure_no_blocking_journal(app_data_dir, &key)?;
        let canonical_message = validate_commit_message(&input.message)?;
        let snapshot = require_snapshot(state, &key, input.write_generation, &input.snapshot_id)?;
        let projection = inspect_repository(binding)?;
        ensure_snapshot_preimage(&snapshot, &projection)?;
        if projection.staged.is_empty() {
            return Err("There are no staged files to commit".to_string());
        }
        let identity = snapshot.identity.clone().ok_or_else(|| {
            "Set repository-local user.name and user.email before committing".to_string()
        })?;
        if projection.identity.as_ref() != Some(&identity) {
            return Err(
                "Repository commit identity changed after the reviewed snapshot".to_string(),
            );
        }
        let message = canonical_commit_message(&canonical_message, &identity)?;
        let receipt = transaction::execute_commit(CommitClaim {
            app_data_dir,
            binding,
            key: key.clone(),
            input_digest,
            write_generation: input.write_generation,
            snapshot_id: input.snapshot_id,
            identity,
            message,
            projection,
        })?;
        advance_write_generation(state, &key)?;
        Ok(receipt)
    })
}

pub(crate) fn reconcile(
    state: &CodeGitWriteState,
    app_data_dir: &Path,
    binding: &CodeThreadBinding,
    input: CodeGitReconcileInput,
) -> Result<CodeGitReconcileResult, String> {
    validate_context(&input.scope, &input.thread_id, binding)?;
    let key = binding_key(&input.scope, &input.thread_id);
    let _repository_guard = state
        .repository_write
        .lock()
        .map_err(|_| "SchoolX Code Git write lock is unavailable".to_string())?;
    let store = GitOperationJournalStore::for_app_data(app_data_dir)?;
    let mut journal = store.load()?;
    let Some(record) = blocking_record(&journal, &key) else {
        return Ok(CodeGitReconcileResult::None {
            scope: input.scope,
            thread_id: input.thread_id,
        });
    };
    let needs_recovery = record.phase != GitJournalPhase::Uncertain
        && !(record.phase == GitJournalPhase::CompletedAwaitingAck && record.cleanup_complete);
    if needs_recovery {
        let record_id = record.record_id.clone();
        transaction::recover_record(app_data_dir, binding, &record_id)?;
        journal = store.load()?;
    }
    let Some(record) = blocking_record(&journal, &key) else {
        return Ok(CodeGitReconcileResult::None {
            scope: input.scope,
            thread_id: input.thread_id,
        });
    };
    match record.phase {
        GitJournalPhase::Prepared => Ok(CodeGitReconcileResult::Pending {
            scope: input.scope,
            thread_id: input.thread_id,
            operation_id: record.operation_id.clone(),
            operation: record.operation,
        }),
        GitJournalPhase::CompletedAwaitingAck if record.cleanup_complete => {
            Ok(CodeGitReconcileResult::Completed {
                receipt: record
                    .receipt
                    .clone()
                    .ok_or_else(|| "Completed Git journal is missing its receipt".to_string())?,
            })
        }
        GitJournalPhase::CompletedAwaitingAck => Ok(CodeGitReconcileResult::Recovering {
            scope: input.scope,
            thread_id: input.thread_id,
            operation_id: record.operation_id.clone(),
            operation: record.operation,
        }),
        GitJournalPhase::Uncertain => Ok(CodeGitReconcileResult::Uncertain {
            scope: input.scope,
            thread_id: input.thread_id,
            operation_id: record.operation_id.clone(),
            operation: record.operation,
            message: record
                .diagnostic
                .clone()
                .unwrap_or_else(|| "Native Git evidence is uncertain".to_string()),
        }),
        GitJournalPhase::Acknowledged => Ok(CodeGitReconcileResult::None {
            scope: input.scope,
            thread_id: input.thread_id,
        }),
        _ => Ok(CodeGitReconcileResult::Recovering {
            scope: input.scope,
            thread_id: input.thread_id,
            operation_id: record.operation_id.clone(),
            operation: record.operation,
        }),
    }
}

pub(crate) fn acknowledge(
    state: &CodeGitWriteState,
    app_data_dir: &Path,
    input: CodeGitAcknowledgeInput,
) -> Result<CodeGitAcknowledgeReceipt, String> {
    let key = binding_key(&input.scope, &input.thread_id);
    let _repository_guard = state
        .repository_write
        .lock()
        .map_err(|_| "SchoolX Code Git write lock is unavailable".to_string())?;
    let store = GitOperationJournalStore::for_app_data(app_data_dir)?;
    let mut journal = store.load()?;
    let record_position = journal
        .records
        .iter()
        .position(|record| record.key == key && record.operation_id == input.operation_id)
        .ok_or_else(|| "Completed Git operation was not found".to_string())?;
    let initial_record = &journal.records[record_position];
    if initial_record.phase == GitJournalPhase::Acknowledged {
        let acknowledgement = initial_record
            .acknowledgement
            .as_ref()
            .ok_or_else(|| "Acknowledged Git operation lost its coordinate".to_string())?;
        if acknowledgement.write_generation != input.write_generation
            || acknowledgement.snapshot_id != input.snapshot_id
        {
            return Err("Acknowledgement retry does not match its durable coordinate".to_string());
        }
        return Ok(CodeGitAcknowledgeReceipt {
            scope: input.scope,
            thread_id: input.thread_id,
            operation_id: input.operation_id,
            disposition: "acknowledged".to_string(),
        });
    }
    super::with_git_authority(|| {
        if journal.records[record_position].phase == GitJournalPhase::CompletedAwaitingAck
            && !journal.records[record_position].cleanup_complete
        {
            let record_id = journal.records[record_position].record_id.clone();
            transaction::complete_record_cleanup(app_data_dir, &record_id)?;
            journal = store.load()?;
        }
        let record = journal
            .records
            .iter_mut()
            .find(|record| record.key == key && record.operation_id == input.operation_id)
            .ok_or_else(|| "Completed Git operation was not found".to_string())?;
        if record.phase != GitJournalPhase::CompletedAwaitingAck {
            return Err("Git operation is not ready to acknowledge".to_string());
        }
        if !record.cleanup_complete {
            return Err("Completed Git operation cleanup is not durable".to_string());
        }
        let receipt = record
            .receipt
            .as_ref()
            .ok_or_else(|| "Completed Git operation is missing its receipt".to_string())?;
        if receipt.request_generation().saturating_add(1) != input.write_generation {
            return Err(
                "Acknowledgement write generation does not prove the post-operation snapshot"
                    .to_string(),
            );
        }
        let snapshot = require_snapshot(state, &key, input.write_generation, &input.snapshot_id)?;
        if snapshot.write_generation != input.write_generation {
            return Err(
                "Acknowledgement snapshot belongs to a different write generation".to_string(),
            );
        }
        let receipt_digest = digest_json(receipt)?;
        record.phase = GitJournalPhase::Acknowledged;
        record.acknowledgement = Some(GitJournalAckCoordinate {
            operation_id: input.operation_id.clone(),
            write_generation: input.write_generation,
            snapshot_id: input.snapshot_id.clone(),
            receipt_digest,
        });
        store.save(&journal)?;
        super::transaction::fault::checkpoint(
            super::transaction::fault::TransactionFaultBoundary::AcknowledgementPersisted,
        )?;
        let mut memory = lock_memory(&state.inner)?;
        if let Some(binding) = memory.bindings.get_mut(&key) {
            binding.status_revision = binding.status_revision.saturating_add(1);
            binding.projection_digest.clear();
            binding.snapshot = None;
        }
        Ok(CodeGitAcknowledgeReceipt {
            scope: input.scope,
            thread_id: input.thread_id,
            operation_id: input.operation_id,
            disposition: "acknowledged".to_string(),
        })
    })
}

/// Refuse any competing bound-thread admission while a durable Git operation
/// is pending, uncertain, or completed but not yet acknowledged.
pub(crate) fn ensure_admission_clear(
    app_data_dir: &Path,
    scope: &CodeThreadBindingScope,
    thread_id: &str,
) -> Result<(), String> {
    ensure_no_blocking_journal(app_data_dir, &binding_key(scope, thread_id))
}

fn ensure_no_blocking_journal(
    app_data_dir: &Path,
    key: &GitJournalBindingKey,
) -> Result<(), String> {
    let journal = GitOperationJournalStore::for_app_data(app_data_dir)?.load()?;
    if blocking_record(&journal, key).is_some() {
        Err("Another Git operation must be reconciled or acknowledged first".to_string())
    } else {
        Ok(())
    }
}

fn exact_retry_receipt(
    app_data_dir: &Path,
    key: &GitJournalBindingKey,
    operation: CodeGitOperation,
    input_digest: &str,
) -> Result<Option<CodeGitMutationReceipt>, String> {
    let store = GitOperationJournalStore::for_app_data(app_data_dir)?;
    let journal = store.load()?;
    let matching = journal.records.iter().rev().find(|record| {
        record.key == *key && record.operation == operation && record.input_digest == input_digest
    });
    let Some(record) = matching else {
        return Ok(None);
    };
    if record.phase == GitJournalPhase::CompletedAwaitingAck && !record.cleanup_complete {
        let record_id = record.record_id.clone();
        transaction::complete_record_cleanup(app_data_dir, &record_id)?;
        let reloaded = store.load()?;
        let completed = reloaded
            .records
            .iter()
            .find(|candidate| candidate.record_id == record_id)
            .ok_or_else(|| "Completed Git retry disappeared during cleanup".to_string())?;
        if !completed.cleanup_complete {
            return Err("Completed Git retry cleanup is not durable".to_string());
        }
        return completed
            .receipt
            .clone()
            .map(Some)
            .ok_or_else(|| "Completed Git retry lost its exact receipt".to_string());
    }
    Ok(record.receipt.clone())
}

fn blocking_record<'a>(
    journal: &'a GitOperationJournal,
    key: &GitJournalBindingKey,
) -> Option<&'a GitJournalRecord> {
    journal
        .records
        .iter()
        .rev()
        .find(|record| record.key == *key && record.phase.is_blocking())
}

fn durable_write_generation(journal: &GitOperationJournal, key: &GitJournalBindingKey) -> u64 {
    journal
        .records
        .iter()
        .filter(|record| record.key == *key)
        .map(|record| {
            record
                .request
                .write_generation
                .saturating_add(u64::from(matches!(
                    record.phase,
                    GitJournalPhase::CompletedAwaitingAck | GitJournalPhase::Acknowledged
                )))
        })
        .max()
        .unwrap_or_default()
}

fn journal_state(record: &GitJournalRecord) -> &'static str {
    match record.phase {
        GitJournalPhase::Prepared => "pending",
        GitJournalPhase::Uncertain => "uncertain",
        GitJournalPhase::CompletedAwaitingAck => "completedAwaitingAck",
        GitJournalPhase::Acknowledged => "acknowledged",
        _ => "recovering",
    }
}

fn require_snapshot(
    state: &CodeGitWriteState,
    key: &GitJournalBindingKey,
    generation: u64,
    snapshot_id: &str,
) -> Result<Snapshot, String> {
    let memory = lock_memory(&state.inner)?;
    let snapshot = memory
        .bindings
        .get(key)
        .and_then(|binding| binding.snapshot.as_ref())
        .ok_or_else(|| "Git snapshot expired; refresh before retrying".to_string())?;
    if snapshot.write_generation != generation || snapshot.id != snapshot_id {
        return Err(
            "Git snapshot coordinate is stale or belongs to another generation".to_string(),
        );
    }
    Ok(snapshot.clone())
}

fn ensure_snapshot_preimage(
    snapshot: &Snapshot,
    projection: &RepoProjection,
) -> Result<(), String> {
    if snapshot.preimage != projection.preimage
        || snapshot.head != projection.head
        || snapshot.root != projection.root
        || snapshot.admin != projection.admin
    {
        return Err(
            "Git workspace changed after the reviewed snapshot; refresh before retrying"
                .to_string(),
        );
    }
    Ok(())
}

fn advance_write_generation(
    state: &CodeGitWriteState,
    key: &GitJournalBindingKey,
) -> Result<(), String> {
    let mut memory = lock_memory(&state.inner)?;
    let binding = memory.bindings.entry(key.clone()).or_default();
    binding.write_generation = binding.write_generation.saturating_add(1);
    binding.status_revision = binding.status_revision.saturating_add(1);
    binding.projection_digest.clear();
    binding.snapshot = None;
    Ok(())
}

fn capability(blocker: Option<String>, available: bool, empty_reason: &str) -> CodeGitCapability {
    if let Some(reason) = blocker {
        CodeGitCapability {
            enabled: false,
            reason: Some(reason),
        }
    } else if available {
        CodeGitCapability {
            enabled: true,
            reason: None,
        }
    } else {
        CodeGitCapability {
            enabled: false,
            reason: Some(empty_reason.to_string()),
        }
    }
}

fn validate_context(
    scope: &CodeThreadBindingScope,
    thread_id: &str,
    binding: &CodeThreadBinding,
) -> Result<(), String> {
    if binding.codex_thread_id != thread_id
        || binding.community_id != scope.community_id
        || binding.project_dtag != scope.project_dtag
        || binding.repository_identity != scope.repository_identity
    {
        return Err("Git command does not match the exact persisted thread binding".to_string());
    }
    Ok(())
}

pub(super) fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.len() > 4096
        || path.starts_with('/')
        || path.split('/').any(|part| part == "..")
        || path.chars().any(|value| value.is_control())
    {
        return Err("Git path cannot be represented safely for whole-file writes".to_string());
    }
    Ok(())
}

pub(super) fn status_from_code(code: char, untracked: bool) -> Result<CodeGitChangeStatus, String> {
    if untracked {
        return Ok(CodeGitChangeStatus::Untracked);
    }
    match code {
        'A' => Ok(CodeGitChangeStatus::Added),
        'M' => Ok(CodeGitChangeStatus::Modified),
        'D' => Ok(CodeGitChangeStatus::Deleted),
        'T' => Ok(CodeGitChangeStatus::TypeChanged),
        'U' => Ok(CodeGitChangeStatus::Unmerged),
        _ => Err(format!("Unsupported Git change status {code}")),
    }
}

fn binding_key(scope: &CodeThreadBindingScope, thread_id: &str) -> GitJournalBindingKey {
    GitJournalBindingKey {
        scope: scope.clone(),
        thread_id: thread_id.to_string(),
    }
}

pub(super) fn identity_digest(identity: Option<&CodeGitCommitIdentity>) -> String {
    identity
        .map(|value| digest_text(&format!("{}\0{}", value.name, value.email)))
        .unwrap_or_default()
}

fn digest_json(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|bytes| digest_bytes(&bytes))
        .map_err(|error| error.to_string())
}

pub(super) fn digest_text(value: &str) -> String {
    digest_bytes(value.as_bytes())
}

pub(super) fn digest_bytes(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn random_hex() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| format!("failed to issue opaque Git coordinate: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn lock_memory(
    memory: &Mutex<MemoryState>,
) -> Result<std::sync::MutexGuard<'_, MemoryState>, String> {
    memory
        .lock()
        .map_err(|_| "SchoolX Code Git snapshot state is unavailable".to_string())
}
