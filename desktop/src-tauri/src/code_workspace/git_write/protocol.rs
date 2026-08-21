use serde::{Deserialize, Serialize};

use crate::code_workspace::CodeThreadBindingScope;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeGitStatusInput {
    pub scope: CodeThreadBindingScope,
    pub thread_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeGitIndexMutationInput {
    pub scope: CodeThreadBindingScope,
    pub thread_id: String,
    pub write_generation: u64,
    pub snapshot_id: String,
    pub file_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeGitCommitInput {
    pub scope: CodeThreadBindingScope,
    pub thread_id: String,
    pub write_generation: u64,
    pub snapshot_id: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeGitReconcileInput {
    pub scope: CodeThreadBindingScope,
    pub thread_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeGitAcknowledgeInput {
    pub scope: CodeThreadBindingScope,
    pub thread_id: String,
    pub operation_id: String,
    pub write_generation: u64,
    pub snapshot_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodeGitOperation {
    Stage,
    Unstage,
    Commit,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodeGitChangeStatus {
    Added,
    Modified,
    Deleted,
    TypeChanged,
    Unmerged,
    Untracked,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeGitChangeFile {
    pub file_id: String,
    pub path: String,
    pub status: CodeGitChangeStatus,
    pub binary: bool,
    pub additions: usize,
    pub deletions: usize,
    pub patch: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeGitChangeSet {
    pub files: Vec<CodeGitChangeFile>,
    pub total_files: usize,
    pub files_truncated: bool,
    pub additions: usize,
    pub deletions: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeGitCommitIdentity {
    pub name: String,
    pub email: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeGitCapability {
    pub enabled: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeGitCapabilities {
    pub stage: CodeGitCapability,
    pub unstage: CodeGitCapability,
    pub commit: CodeGitCapability,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeGitIndexMutationReceipt {
    pub operation_id: String,
    pub operation: CodeGitOperation,
    pub scope: CodeThreadBindingScope,
    pub thread_id: String,
    pub request_generation: u64,
    pub before_snapshot_id: String,
    pub file_id: String,
    pub disposition: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeGitCommitReceipt {
    pub operation_id: String,
    pub operation: CodeGitOperation,
    pub scope: CodeThreadBindingScope,
    pub thread_id: String,
    pub request_generation: u64,
    pub before_snapshot_id: String,
    pub previous_head: String,
    pub commit: String,
    pub tree: String,
    pub disposition: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum CodeGitMutationReceipt {
    Index(CodeGitIndexMutationReceipt),
    Commit(CodeGitCommitReceipt),
}

impl CodeGitMutationReceipt {
    pub(crate) fn operation_id(&self) -> &str {
        match self {
            Self::Index(value) => &value.operation_id,
            Self::Commit(value) => &value.operation_id,
        }
    }

    pub(crate) fn request_generation(&self) -> u64 {
        match self {
            Self::Index(value) => value.request_generation,
            Self::Commit(value) => value.request_generation,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CodeGitStatus {
    Ready {
        runtime_generation: u64,
        status_revision: u64,
        write_generation: u64,
        snapshot_sequence: u64,
        scope: CodeThreadBindingScope,
        thread_id: String,
        snapshot_id: String,
        head_commit: String,
        task: Box<CodeGitChangeSet>,
        staged: Box<CodeGitChangeSet>,
        unstaged: Box<CodeGitChangeSet>,
        has_conflicts: bool,
        commit_identity: Option<CodeGitCommitIdentity>,
        capabilities: Box<CodeGitCapabilities>,
        blocking_receipt: Option<Box<CodeGitMutationReceipt>>,
    },
    Blocked {
        runtime_generation: u64,
        status_revision: u64,
        write_generation: u64,
        scope: CodeThreadBindingScope,
        thread_id: String,
        reason: String,
        remediation: String,
    },
    RecoveryRequired {
        runtime_generation: u64,
        status_revision: u64,
        write_generation: u64,
        scope: CodeThreadBindingScope,
        thread_id: String,
        operation: CodeGitRecoveryOperation,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeGitRecoveryOperation {
    pub operation_id: String,
    pub operation: CodeGitOperation,
    pub journal_state: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CodeGitReconcileResult {
    None {
        scope: CodeThreadBindingScope,
        thread_id: String,
    },
    Pending {
        scope: CodeThreadBindingScope,
        thread_id: String,
        operation_id: String,
        operation: CodeGitOperation,
    },
    Recovering {
        scope: CodeThreadBindingScope,
        thread_id: String,
        operation_id: String,
        operation: CodeGitOperation,
    },
    Completed {
        receipt: CodeGitMutationReceipt,
    },
    Uncertain {
        scope: CodeThreadBindingScope,
        thread_id: String,
        operation_id: String,
        operation: CodeGitOperation,
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeGitAcknowledgeReceipt {
    pub scope: CodeThreadBindingScope,
    pub thread_id: String,
    pub operation_id: String,
    pub disposition: String,
}
