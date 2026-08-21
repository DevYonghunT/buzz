//! Strict durable transaction journal for SchoolX Code Git writes.
//!
//! This store is intentionally independent from the binding/removal store and
//! from the public Tauri DTOs. Transaction and recovery code will consume these
//! crate-private types once the pinned publish engine is wired to them.

#![allow(dead_code)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::protocol::{CodeGitCommitIdentity, CodeGitMutationReceipt, CodeGitOperation};
use crate::code_workspace::CodeThreadBindingScope;

pub(crate) const GIT_OPERATION_JOURNAL_VERSION: u32 = 1;
pub(crate) const MAX_GIT_OPERATION_JOURNAL_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const MAX_GIT_OPERATION_JOURNAL_RECORDS: usize = 4_096;
pub(crate) const MAX_ACKNOWLEDGED_TOMBSTONES: usize = 256;

const JOURNAL_DIRECTORY: &str = "code";
const JOURNAL_FILE: &str = "git-operations.json";
const MAX_IDENTIFIER_BYTES: usize = 512;
const MAX_PATH_BYTES: usize = 32 * 1024;
const MAX_GIT_PATH_BYTES: usize = 4_096;
const MAX_ARTIFACT_NAME_BYTES: usize = 255;
const MAX_DIAGNOSTIC_BYTES: usize = 4 * 1024;
const MAX_IDENTITY_NAME_BYTES: usize = 512;
const MAX_IDENTITY_EMAIL_BYTES: usize = 512;
const MAX_COMMIT_MESSAGE_BYTES: u64 = 64 * 1024;
const MAX_PRIVATE_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;

/// Exact binding coordinate used for blocker joins with the safe-remove store.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GitJournalBindingKey {
    pub(crate) scope: CodeThreadBindingScope,
    pub(crate) thread_id: String,
}

impl GitJournalBindingKey {
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.scope.validate()?;
        validate_identifier("Git journal thread", &self.thread_id)
    }
}

/// Durable transaction boundary. Optional evidence in a record permits the
/// record to be synced between substeps without inventing an unproven phase.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum GitJournalPhase {
    Prepared,
    ObjectWritten,
    ArtifactsReady,
    LocksReady,
    IndexPublished,
    HeadPublished,
    CompletedAwaitingAck,
    Acknowledged,
    Uncertain,
}

impl GitJournalPhase {
    pub(crate) fn is_blocking(self) -> bool {
        self != Self::Acknowledged
    }
}

/// Immutable public-request coordinate, excluding caller-controlled paths,
/// refs, identities, and operation ids.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GitJournalRequestCoordinate {
    pub(crate) write_generation: u64,
    pub(crate) snapshot_id: String,
    pub(crate) file_id: Option<String>,
}

impl GitJournalRequestCoordinate {
    fn validate(&self, operation: CodeGitOperation) -> Result<(), String> {
        self.write_generation.checked_add(1).ok_or_else(|| {
            "Git journal request generation cannot advance past u64::MAX".to_string()
        })?;
        validate_hex_64("Git journal snapshot id", &self.snapshot_id)?;
        match (operation, self.file_id.as_deref()) {
            (CodeGitOperation::Stage | CodeGitOperation::Unstage, Some(file_id)) => {
                validate_hex_64("Git journal file id", file_id)
            }
            (CodeGitOperation::Stage | CodeGitOperation::Unstage, None) => {
                Err("Git index journal request is missing its opaque file id".to_string())
            }
            (CodeGitOperation::Commit, None) => Ok(()),
            (CodeGitOperation::Commit, Some(_)) => {
                Err("Git commit journal request cannot carry a file id".to_string())
            }
        }
    }
}

/// Stable filesystem identity for a pinned repository path.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GitJournalPathIdentity {
    pub(crate) exact_path: String,
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) owner: u32,
    pub(crate) mode: u32,
    pub(crate) link_count: u64,
}

/// Stable content and filesystem identity for the pinned Git executable.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GitJournalFileIdentity {
    pub(crate) exact_path: String,
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) owner: u32,
    pub(crate) mode: u32,
    pub(crate) link_count: u64,
    pub(crate) size: u64,
    pub(crate) sha256: String,
}

/// Frozen repository and tool authority shared by every transaction phase.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GitJournalRepositoryEvidence {
    pub(crate) repository_identity: String,
    pub(crate) root: GitJournalPathIdentity,
    pub(crate) admin: GitJournalPathIdentity,
    pub(crate) common_dir: GitJournalPathIdentity,
    pub(crate) object_database: GitJournalPathIdentity,
    pub(crate) worktree_git_file: GitJournalFileIdentity,
    pub(crate) git_executable: GitJournalFileIdentity,
    pub(crate) previous_head: String,
    pub(crate) head_file_digest: String,
    pub(crate) before_index_digest: String,
    pub(crate) before_config_digest: String,
}

impl GitJournalRepositoryEvidence {
    fn validate(&self, key: &GitJournalBindingKey) -> Result<(), String> {
        validate_sha256("Git journal repository identity", &self.repository_identity)?;
        if self.repository_identity != key.scope.repository_identity {
            return Err("Git journal repository evidence does not match its binding".to_string());
        }
        self.root.validate_directory("Git journal root")?;
        self.admin
            .validate_directory("Git journal admin directory")?;
        self.common_dir
            .validate_directory("Git journal common directory")?;
        self.object_database
            .validate_directory("Git journal object database")?;
        self.worktree_git_file
            .validate_singly_linked_regular_file("Git journal worktree .git file", 32 * 1024)?;
        self.git_executable
            .validate_executable("Git journal executable")?;
        if self.root.exact_path == self.admin.exact_path
            || self.root.exact_path == self.common_dir.exact_path
            || self.admin.exact_path == self.common_dir.exact_path
        {
            return Err("Git journal repository paths are not distinct".to_string());
        }
        if Path::new(&self.worktree_git_file.exact_path)
            != Path::new(&self.root.exact_path).join(".git")
            || Path::new(&self.object_database.exact_path)
                != Path::new(&self.common_dir.exact_path).join("objects")
        {
            return Err("Git journal repository authority paths are inconsistent".to_string());
        }
        if Path::new(&self.admin.exact_path).parent()
            != Some(
                Path::new(&self.common_dir.exact_path)
                    .join("worktrees")
                    .as_path(),
            )
        {
            return Err(
                "Git journal admin escaped the exact common-dir/worktrees boundary".to_string(),
            );
        }
        if self.worktree_git_file.owner != self.root.owner
            || self.object_database.owner != self.common_dir.owner
        {
            return Err("Git journal repository authority owner changed".to_string());
        }
        validate_object_id("Git journal previous HEAD", &self.previous_head, false)?;
        validate_sha256("Git journal HEAD-file digest", &self.head_file_digest)?;
        if self.head_file_digest != digest_bytes(format!("{}\n", self.previous_head).as_bytes()) {
            return Err("Git journal HEAD-file digest does not match detached HEAD".to_string());
        }
        validate_sha256("Git journal before-index digest", &self.before_index_digest)?;
        validate_sha256("Git journal config digest", &self.before_config_digest)
    }
}

/// Exact regular-file evidence for an app-private candidate, frozen source or
/// message, or a proven-owned artifact in the linked-worktree admin directory.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GitJournalArtifactEvidence {
    pub(crate) parent_path: String,
    pub(crate) name: String,
    pub(crate) device: u64,
    pub(crate) inode: u64,
    pub(crate) owner: u32,
    pub(crate) mode: u32,
    pub(crate) link_count: u64,
    pub(crate) size: u64,
    pub(crate) sha256: String,
}

impl GitJournalArtifactEvidence {
    fn validate(&self, label: &str) -> Result<(), String> {
        validate_absolute_path(&format!("{label} parent"), &self.parent_path)?;
        validate_component(&format!("{label} name"), &self.name)?;
        if self.device == 0
            || self.inode == 0
            || self.link_count == 0
            || self.size > MAX_PRIVATE_ARTIFACT_BYTES
        {
            return Err(format!("{label} has an invalid filesystem identity"));
        }
        validate_regular_file_mode(label, self.mode)?;
        validate_sha256(&format!("{label} digest"), &self.sha256)
    }

    fn validate_private(&self, label: &str, expected_owner: u32) -> Result<(), String> {
        self.validate(label)?;
        if self.owner != expected_owner || self.mode & 0o7777 != 0o600 {
            return Err(format!(
                "{label} must be an owner-matched private mode-0600 artifact"
            ));
        }
        Ok(())
    }

    fn exact_path(&self) -> PathBuf {
        Path::new(&self.parent_path).join(&self.name)
    }
}

/// Artifact slots are fixed so unknown or accidentally omitted evidence cannot
/// be hidden by a map entry or an ad-hoc filename.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GitJournalArtifactSet {
    pub(crate) candidate_index: GitJournalArtifactEvidence,
    pub(crate) source: Option<GitJournalArtifactEvidence>,
    pub(crate) message: Option<GitJournalArtifactEvidence>,
    pub(crate) index_artifact_name: String,
    pub(crate) head_artifact_name: String,
    pub(crate) index_artifact: Option<GitJournalArtifactEvidence>,
    pub(crate) head_artifact: Option<GitJournalArtifactEvidence>,
}

impl GitJournalArtifactSet {
    fn validate_common(&self, repository: &GitJournalRepositoryEvidence) -> Result<(), String> {
        self.candidate_index
            .validate_private("Git journal candidate index", repository.admin.owner)?;
        if matches!(
            self.candidate_index.parent_path.as_str(),
            path if path == repository.root.exact_path
                || path == repository.admin.exact_path
                || path == repository.common_dir.exact_path
        ) {
            return Err("Git journal candidate index is not app-private".to_string());
        }
        if self.candidate_index.link_count != 1 || self.candidate_index.size == 0 {
            return Err(
                "Git journal candidate index must be non-empty and record link-count one"
                    .to_string(),
            );
        }
        if let Some(source) = &self.source {
            source.validate_private("Git journal source", repository.admin.owner)?;
            if source.link_count != 1 {
                return Err("Git journal source must record link-count one".to_string());
            }
            if source.parent_path != self.candidate_index.parent_path
                || source.name == self.candidate_index.name
                || (source.device == self.candidate_index.device
                    && source.inode == self.candidate_index.inode)
            {
                return Err(
                    "Git journal frozen source must use the private candidate artifact root"
                        .to_string(),
                );
            }
        }
        if let Some(message) = &self.message {
            message.validate_private("Git journal message", repository.admin.owner)?;
            if message.link_count != 1
                || message.size == 0
                || message.size > MAX_COMMIT_MESSAGE_BYTES
            {
                return Err(
                    "Git journal message artifact is not bounded and singly linked".to_string(),
                );
            }
            if message.parent_path != self.candidate_index.parent_path
                || message.name == self.candidate_index.name
                || self.source.as_ref().is_some_and(|source| {
                    source.name == message.name
                        || (source.device == message.device && source.inode == message.inode)
                })
                || (message.device == self.candidate_index.device
                    && message.inode == self.candidate_index.inode)
            {
                return Err(
                    "Git journal message must be a distinct private candidate-root artifact"
                        .to_string(),
                );
            }
        }
        if let Some(index) = &self.index_artifact {
            index.validate_private("Git journal index artifact", repository.admin.owner)?;
        }
        if let Some(head) = &self.head_artifact {
            head.validate_private("Git journal HEAD artifact", repository.admin.owner)?;
        }
        validate_component(
            "Git journal planned index artifact name",
            &self.index_artifact_name,
        )?;
        validate_component(
            "Git journal planned HEAD artifact name",
            &self.head_artifact_name,
        )?;
        if self.index_artifact_name == self.head_artifact_name
            || matches!(
                self.index_artifact_name.as_str(),
                "index" | "index.lock" | "HEAD" | "HEAD.lock"
            )
            || matches!(
                self.head_artifact_name.as_str(),
                "index" | "index.lock" | "HEAD" | "HEAD.lock"
            )
        {
            return Err("Git journal planned admin artifact name is ambiguous".to_string());
        }
        Ok(())
    }

    fn validate_admin_artifacts(
        &self,
        phase: GitJournalPhase,
        repository: &GitJournalRepositoryEvidence,
    ) -> Result<(), String> {
        let requires_artifacts = matches!(
            phase,
            GitJournalPhase::ArtifactsReady
                | GitJournalPhase::LocksReady
                | GitJournalPhase::IndexPublished
                | GitJournalPhase::HeadPublished
                | GitJournalPhase::CompletedAwaitingAck
                | GitJournalPhase::Acknowledged
        );
        let forbids_artifacts = matches!(
            phase,
            GitJournalPhase::Prepared | GitJournalPhase::ObjectWritten
        );
        match (&self.index_artifact, &self.head_artifact) {
            (Some(index), Some(head)) => {
                if forbids_artifacts {
                    return Err(format!(
                        "Git journal phase {phase:?} cannot carry owned admin artifacts"
                    ));
                }
                if index.parent_path != repository.admin.exact_path
                    || head.parent_path != repository.admin.exact_path
                {
                    return Err(
                        "Git journal owned artifacts escaped the pinned admin directory"
                            .to_string(),
                    );
                }
                if index.name != self.index_artifact_name || head.name != self.head_artifact_name {
                    return Err(
                        "Git journal admin artifact evidence does not match its prepared names"
                            .to_string(),
                    );
                }
                if (index.device == head.device && index.inode == head.inode)
                    || index.link_count != head.link_count
                    || !matches!(index.link_count, 1 | 2)
                {
                    return Err(
                        "Git journal admin artifacts have ambiguous inode/link evidence"
                            .to_string(),
                    );
                }
                let expected_links = if phase == GitJournalPhase::ArtifactsReady {
                    1
                } else if matches!(
                    phase,
                    GitJournalPhase::LocksReady
                        | GitJournalPhase::IndexPublished
                        | GitJournalPhase::HeadPublished
                        | GitJournalPhase::CompletedAwaitingAck
                        | GitJournalPhase::Acknowledged
                ) {
                    2
                } else {
                    index.link_count
                };
                if requires_artifacts
                    && (index.link_count != expected_links || head.link_count != expected_links)
                {
                    return Err(format!(
                        "Git journal phase {phase:?} has the wrong owned-artifact link count"
                    ));
                }
            }
            (None, None) if !requires_artifacts => {}
            (None, None) => {
                return Err(format!(
                    "Git journal phase {phase:?} is missing its owned admin artifacts"
                ));
            }
            _ => {
                return Err(
                    "Git journal must record both owned admin artifacts together".to_string(),
                );
            }
        }
        Ok(())
    }
}

/// Expected candidate-index semantics for stage and unstage.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GitJournalIndexTransaction {
    pub(crate) expected_index_digest: String,
    pub(crate) expected_semantic_digest: String,
    pub(crate) selected_path: String,
    pub(crate) selected_mode: String,
    pub(crate) selected_blob_oid: String,
}

impl GitJournalIndexTransaction {
    fn validate(&self, object_id_length: usize) -> Result<(), String> {
        validate_sha256(
            "Git journal expected-index digest",
            &self.expected_index_digest,
        )?;
        validate_sha256(
            "Git journal expected semantic digest",
            &self.expected_semantic_digest,
        )?;
        validate_relative_git_path(&self.selected_path)?;
        if !matches!(self.selected_mode.as_str(), "000000" | "100644" | "100755") {
            return Err("Git journal selected mode is unsupported".to_string());
        }
        validate_object_id_with_length(
            "Git journal selected blob",
            &self.selected_blob_oid,
            object_id_length,
            self.selected_mode == "000000",
        )?;
        if self.selected_mode == "000000"
            && !self.selected_blob_oid.bytes().all(|byte| byte == b'0')
        {
            return Err("Git journal deleted entry must use the zero blob id".to_string());
        }
        if self.selected_mode != "000000" && self.selected_blob_oid.bytes().all(|byte| byte == b'0')
        {
            return Err("Git journal regular entry cannot use the zero blob id".to_string());
        }
        Ok(())
    }
}

/// Canonical Git identity timestamp; both the seconds and numeric UTC offset
/// are frozen before object creation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GitJournalTimestamp {
    pub(crate) unix_seconds: i64,
    pub(crate) offset_minutes: i16,
}

impl GitJournalTimestamp {
    fn validate(self) -> Result<(), String> {
        if self.unix_seconds < 0 || !(-1_439..=1_439).contains(&self.offset_minutes) {
            return Err("Git journal timestamp is outside the supported range".to_string());
        }
        Ok(())
    }
}

/// Frozen identity, timestamps, message, and eventual object ids for commit.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GitJournalCommitTransaction {
    pub(crate) identity: CodeGitCommitIdentity,
    pub(crate) author_timestamp: GitJournalTimestamp,
    pub(crate) committer_timestamp: GitJournalTimestamp,
    pub(crate) index_semantic_digest: String,
    pub(crate) message_digest: String,
    pub(crate) tree_oid: Option<String>,
    pub(crate) commit_oid: Option<String>,
}

impl GitJournalCommitTransaction {
    fn validate(&self, object_id_length: usize) -> Result<(), String> {
        validate_commit_identity(&self.identity)?;
        self.author_timestamp.validate()?;
        self.committer_timestamp.validate()?;
        validate_sha256(
            "Git journal frozen-index semantic digest",
            &self.index_semantic_digest,
        )?;
        validate_sha256("Git journal canonical-message digest", &self.message_digest)?;
        if self.commit_oid.is_some() && self.tree_oid.is_none() {
            return Err("Git journal commit object is missing its tree object".to_string());
        }
        if let Some(tree) = &self.tree_oid {
            validate_object_id_with_length(
                "Git journal tree object",
                tree,
                object_id_length,
                false,
            )?;
        }
        if let Some(commit) = &self.commit_oid {
            validate_object_id_with_length(
                "Git journal commit object",
                commit,
                object_id_length,
                false,
            )?;
        }
        Ok(())
    }
}

/// Durable acknowledgement coordinate bound to the exact completed receipt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GitJournalAckCoordinate {
    pub(crate) operation_id: String,
    pub(crate) write_generation: u64,
    pub(crate) snapshot_id: String,
    pub(crate) receipt_digest: String,
}

/// One exact mutation transaction or bounded acknowledged tombstone.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GitJournalRecord {
    pub(crate) record_id: String,
    pub(crate) operation_id: String,
    pub(crate) key: GitJournalBindingKey,
    pub(crate) operation: CodeGitOperation,
    pub(crate) phase: GitJournalPhase,
    pub(crate) request: GitJournalRequestCoordinate,
    pub(crate) input_digest: String,
    pub(crate) repository: GitJournalRepositoryEvidence,
    pub(crate) artifacts: GitJournalArtifactSet,
    pub(crate) index_transaction: Option<GitJournalIndexTransaction>,
    pub(crate) commit_transaction: Option<GitJournalCommitTransaction>,
    pub(crate) receipt: Option<CodeGitMutationReceipt>,
    pub(crate) acknowledgement: Option<GitJournalAckCoordinate>,
    pub(crate) diagnostic: Option<String>,
    pub(crate) recovery_started: bool,
    pub(crate) cleanup_complete: bool,
}

impl GitJournalRecord {
    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_hex_64("Git journal record id", &self.record_id)?;
        validate_hex_64("Git journal operation id", &self.operation_id)?;
        self.key.validate()?;
        self.request.validate(self.operation)?;
        validate_sha256("Git journal input digest", &self.input_digest)?;
        self.repository.validate(&self.key)?;
        self.artifacts.validate_common(&self.repository)?;
        self.artifacts
            .validate_admin_artifacts(self.phase, &self.repository)?;

        let object_id_length = self.repository.previous_head.len();
        match self.operation {
            CodeGitOperation::Stage | CodeGitOperation::Unstage => {
                if self.phase == GitJournalPhase::HeadPublished {
                    return Err("Git index journal uses a commit-only phase".to_string());
                }
                let transaction = self.index_transaction.as_ref().ok_or_else(|| {
                    "Git index journal is missing its index transaction evidence".to_string()
                })?;
                transaction.validate(object_id_length)?;
                if self.phase == GitJournalPhase::ObjectWritten
                    && (self.operation != CodeGitOperation::Stage
                        || transaction.selected_mode == "000000")
                {
                    return Err(
                        "Only a non-deletion stage may record the object-written phase".to_string(),
                    );
                }
                if self.commit_transaction.is_some() {
                    return Err("Git index journal cannot carry commit evidence".to_string());
                }
                self.validate_index_artifacts(transaction)?;
            }
            CodeGitOperation::Commit => {
                if self.phase == GitJournalPhase::IndexPublished {
                    return Err("Git commit journal uses an index-only phase".to_string());
                }
                let transaction = self.commit_transaction.as_ref().ok_or_else(|| {
                    "Git commit journal is missing its commit transaction evidence".to_string()
                })?;
                transaction.validate(object_id_length)?;
                if self.index_transaction.is_some() {
                    return Err(
                        "Git commit journal cannot carry index transaction evidence".to_string()
                    );
                }
                self.validate_commit_artifacts(transaction)?;
                let object_phase = matches!(
                    self.phase,
                    GitJournalPhase::ObjectWritten
                        | GitJournalPhase::ArtifactsReady
                        | GitJournalPhase::LocksReady
                        | GitJournalPhase::HeadPublished
                        | GitJournalPhase::CompletedAwaitingAck
                        | GitJournalPhase::Acknowledged
                );
                if object_phase
                    && (transaction.tree_oid.is_none() || transaction.commit_oid.is_none())
                {
                    return Err(format!(
                        "Git commit journal phase {:?} is missing frozen object ids",
                        self.phase
                    ));
                }
                if self.phase == GitJournalPhase::Prepared && transaction.commit_oid.is_some() {
                    return Err(
                        "Prepared Git commit journal cannot carry a commit object".to_string()
                    );
                }
            }
        }

        self.validate_completion()
    }

    fn validate_index_artifacts(
        &self,
        transaction: &GitJournalIndexTransaction,
    ) -> Result<(), String> {
        if self.artifacts.candidate_index.sha256 != transaction.expected_index_digest {
            return Err("Git candidate index does not match the expected index digest".to_string());
        }
        match self.operation {
            CodeGitOperation::Stage if transaction.selected_mode != "000000" => {
                let source = self.artifacts.source.as_ref().ok_or_else(|| {
                    "Git stage journal is missing its frozen source artifact".to_string()
                })?;
                if source.parent_path != self.artifacts.candidate_index.parent_path {
                    return Err(
                        "Git stage source escaped the private candidate artifact root".to_string(),
                    );
                }
            }
            CodeGitOperation::Stage => {
                if self.artifacts.source.is_some() {
                    return Err("Git deletion stage cannot carry a source artifact".to_string());
                }
            }
            CodeGitOperation::Unstage => {
                if self.artifacts.source.is_some() {
                    return Err("Git unstage journal cannot carry a worktree source".to_string());
                }
            }
            CodeGitOperation::Commit => {
                return Err("Git commit journal reached index-artifact validation".to_string());
            }
        }
        if self.artifacts.message.is_some() {
            return Err("Git index journal cannot carry a commit message artifact".to_string());
        }
        if let Some(index) = &self.artifacts.index_artifact {
            if index.sha256 != transaction.expected_index_digest
                || index.size != self.artifacts.candidate_index.size
            {
                return Err("Git owned index artifact has the wrong digest".to_string());
            }
        }
        if let Some(head) = &self.artifacts.head_artifact {
            let expected = digest_bytes(format!("{}\n", self.repository.previous_head).as_bytes());
            if head.sha256 != expected
                || head.size != (self.repository.previous_head.len() + 1) as u64
            {
                return Err("Git HEAD guard artifact has the wrong digest".to_string());
            }
        }
        Ok(())
    }

    fn validate_commit_artifacts(
        &self,
        transaction: &GitJournalCommitTransaction,
    ) -> Result<(), String> {
        if self.artifacts.source.is_some() {
            return Err("Git commit journal cannot carry a worktree source artifact".to_string());
        }
        if transaction.tree_oid.is_none()
            && self.artifacts.candidate_index.sha256 != self.repository.before_index_digest
        {
            return Err(
                "Git pre-tree commit index does not match its before-index digest".to_string(),
            );
        }
        let message = self.artifacts.message.as_ref().ok_or_else(|| {
            "Git commit journal is missing its canonical message artifact".to_string()
        })?;
        if message.sha256 != transaction.message_digest {
            return Err("Git commit message artifact has the wrong digest".to_string());
        }
        if let Some(index) = &self.artifacts.index_artifact {
            if index.sha256 != self.artifacts.candidate_index.sha256
                || index.size != self.artifacts.candidate_index.size
            {
                return Err("Git commit index guard has the wrong digest".to_string());
            }
        }
        if let Some(head) = &self.artifacts.head_artifact {
            let commit = transaction.commit_oid.as_deref().ok_or_else(|| {
                "Git commit HEAD artifact exists before its commit object".to_string()
            })?;
            let expected = digest_bytes(format!("{commit}\n").as_bytes());
            if head.sha256 != expected || head.size != (commit.len() + 1) as u64 {
                return Err("Git commit HEAD artifact has the wrong digest".to_string());
            }
        }
        Ok(())
    }

    fn validate_completion(&self) -> Result<(), String> {
        let completed = matches!(
            self.phase,
            GitJournalPhase::CompletedAwaitingAck | GitJournalPhase::Acknowledged
        );
        if completed {
            let receipt = self
                .receipt
                .as_ref()
                .ok_or_else(|| "Completed Git journal is missing its exact receipt".to_string())?;
            self.validate_receipt(receipt)?;
        } else if self.receipt.is_some() {
            return Err("Uncompleted Git journal cannot carry a receipt".to_string());
        }
        if self.cleanup_complete && !completed {
            return Err("Only a completed Git journal may record finished cleanup".to_string());
        }
        if self.phase == GitJournalPhase::Acknowledged && !self.cleanup_complete {
            return Err("Acknowledged Git journal is missing durable cleanup proof".to_string());
        }

        match (self.phase, self.acknowledgement.as_ref()) {
            (GitJournalPhase::Acknowledged, Some(ack)) => {
                validate_hex_64("Git acknowledgement operation id", &ack.operation_id)?;
                validate_hex_64("Git acknowledgement snapshot id", &ack.snapshot_id)?;
                validate_sha256("Git acknowledgement receipt digest", &ack.receipt_digest)?;
                if ack.operation_id != self.operation_id {
                    return Err("Git acknowledgement operation id does not match".to_string());
                }
                let expected_generation =
                    self.request
                        .write_generation
                        .checked_add(1)
                        .ok_or_else(|| {
                            "Git acknowledgement generation cannot advance past u64::MAX"
                                .to_string()
                        })?;
                if ack.write_generation != expected_generation
                    || ack.snapshot_id == self.request.snapshot_id
                {
                    return Err(
                        "Git acknowledgement generation does not prove post-state".to_string()
                    );
                }
                let receipt = self.receipt.as_ref().ok_or_else(|| {
                    "Git acknowledgement is missing its completed receipt".to_string()
                })?;
                if ack.receipt_digest != receipt_digest(receipt)? {
                    return Err("Git acknowledgement receipt digest does not match".to_string());
                }
            }
            (GitJournalPhase::Acknowledged, None) => {
                return Err("Acknowledged Git journal is missing its coordinate".to_string());
            }
            (_, Some(_)) => {
                return Err(
                    "Unacknowledged Git journal cannot carry an acknowledgement".to_string()
                );
            }
            (_, None) => {}
        }

        match (self.phase, self.diagnostic.as_deref()) {
            (GitJournalPhase::Uncertain, Some(message)) => {
                validate_bounded_text("Git journal uncertainty", message, MAX_DIAGNOSTIC_BYTES)
            }
            (GitJournalPhase::Uncertain, None) => {
                Err("Uncertain Git journal is missing its bounded diagnostic".to_string())
            }
            (_, Some(_)) => Err("Only an uncertain Git journal may carry a diagnostic".to_string()),
            (_, None) => Ok(()),
        }
    }

    fn validate_receipt(&self, receipt: &CodeGitMutationReceipt) -> Result<(), String> {
        match (self.operation, receipt) {
            (
                CodeGitOperation::Stage | CodeGitOperation::Unstage,
                CodeGitMutationReceipt::Index(value),
            ) => {
                if value.operation_id != self.operation_id
                    || value.operation != self.operation
                    || value.scope != self.key.scope
                    || value.thread_id != self.key.thread_id
                    || value.request_generation != self.request.write_generation
                    || value.before_snapshot_id != self.request.snapshot_id
                    || Some(value.file_id.as_str()) != self.request.file_id.as_deref()
                    || value.disposition
                        != if self.operation == CodeGitOperation::Stage {
                            "staged"
                        } else {
                            "unstaged"
                        }
                {
                    return Err(
                        "Git index receipt does not match its immutable journal".to_string()
                    );
                }
                Ok(())
            }
            (CodeGitOperation::Commit, CodeGitMutationReceipt::Commit(value)) => {
                let transaction = self.commit_transaction.as_ref().ok_or_else(|| {
                    "Git commit receipt is missing transaction evidence".to_string()
                })?;
                if value.operation_id != self.operation_id
                    || value.operation != CodeGitOperation::Commit
                    || value.scope != self.key.scope
                    || value.thread_id != self.key.thread_id
                    || value.request_generation != self.request.write_generation
                    || value.before_snapshot_id != self.request.snapshot_id
                    || value.previous_head != self.repository.previous_head
                    || Some(value.tree.as_str()) != transaction.tree_oid.as_deref()
                    || Some(value.commit.as_str()) != transaction.commit_oid.as_deref()
                    || value.disposition != "committed"
                {
                    return Err(
                        "Git commit receipt does not match its immutable journal".to_string()
                    );
                }
                Ok(())
            }
            _ => Err("Git journal receipt belongs to a different operation".to_string()),
        }
    }
}

/// Versioned file content. Record order is insertion order so acknowledged
/// history can be bounded by dropping only the oldest tombstones.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct GitOperationJournal {
    pub(crate) version: u32,
    pub(crate) records: Vec<GitJournalRecord>,
}

impl Default for GitOperationJournal {
    fn default() -> Self {
        Self {
            version: GIT_OPERATION_JOURNAL_VERSION,
            records: Vec::new(),
        }
    }
}

impl GitOperationJournal {
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.validate_records(true)
    }

    fn validate_for_compaction(&self) -> Result<(), String> {
        self.validate_records(false)
    }

    fn validate_records(&self, enforce_tombstone_limit: bool) -> Result<(), String> {
        if self.version != GIT_OPERATION_JOURNAL_VERSION {
            return Err(format!(
                "unsupported SchoolX Code Git journal version {}",
                self.version
            ));
        }
        if self.records.len() > MAX_GIT_OPERATION_JOURNAL_RECORDS {
            return Err(format!(
                "SchoolX Code Git journal exceeds the {MAX_GIT_OPERATION_JOURNAL_RECORDS}-record limit"
            ));
        }

        let mut record_ids = HashSet::with_capacity(self.records.len());
        let mut operation_ids = HashSet::with_capacity(self.records.len());
        let mut exact_inputs = HashSet::with_capacity(self.records.len());
        let mut blocking_bindings = HashSet::new();
        let mut acknowledged = 0_usize;
        for record in &self.records {
            record.validate()?;
            if !record_ids.insert(record.record_id.as_str()) {
                return Err("SchoolX Code Git journal contains a duplicate record id".to_string());
            }
            if !operation_ids.insert(record.operation_id.as_str()) {
                return Err(
                    "SchoolX Code Git journal contains a duplicate operation id".to_string()
                );
            }
            if !exact_inputs.insert((&record.key, record.input_digest.as_str())) {
                return Err(
                    "SchoolX Code Git journal contains a duplicate immutable request".to_string(),
                );
            }
            if record.phase.is_blocking() && !blocking_bindings.insert(&record.key) {
                return Err(
                    "SchoolX Code Git journal contains multiple blockers for one binding"
                        .to_string(),
                );
            }
            if record.phase == GitJournalPhase::Acknowledged {
                acknowledged = acknowledged.saturating_add(1);
            }
        }
        if enforce_tombstone_limit && acknowledged > MAX_ACKNOWLEDGED_TOMBSTONES {
            return Err(format!(
                "SchoolX Code Git journal exceeds the {MAX_ACKNOWLEDGED_TOMBSTONES}-tombstone history limit"
            ));
        }
        Ok(())
    }

    pub(crate) fn blocking_keys(&self) -> HashSet<GitJournalBindingKey> {
        self.records
            .iter()
            .filter(|record| record.phase.is_blocking())
            .map(|record| record.key.clone())
            .collect()
    }

    fn compact_acknowledged(&mut self) {
        let acknowledged = self
            .records
            .iter()
            .filter(|record| record.phase == GitJournalPhase::Acknowledged)
            .count();
        let mut discard = acknowledged.saturating_sub(MAX_ACKNOWLEDGED_TOMBSTONES);
        self.records.retain(|record| {
            if discard > 0 && record.phase == GitJournalPhase::Acknowledged {
                discard -= 1;
                false
            } else {
                true
            }
        });
    }
}

mod store;

#[allow(unused_imports)]
pub(crate) use store::GitOperationJournalStore;
mod validation;

use validation::*;
fn random_hex() -> Result<String, String> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| format!("failed to issue Git journal temp identity: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(test)]
mod tests;
