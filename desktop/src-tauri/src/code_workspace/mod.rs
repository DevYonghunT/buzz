//! Local Codex app-server integration for SchoolX Code.
//!
//! This module is deliberately separate from `managed_agents`: managed agents
//! are relay-facing ACP harnesses, while this runtime is an interactive local
//! client with explicit user approvals.

mod approvals;
pub(crate) mod bindings;
#[cfg(test)]
mod contract_tests;
mod discovery;
#[cfg(target_os = "linux")]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) mod git_launch;
#[cfg(test)]
mod git_launch_contract_tests;
pub(crate) mod git_write;
mod jsonrpc;
#[cfg(target_os = "macos")]
pub(crate) mod macos_git_xpc;
mod model_catalog;
mod paths;
mod protocol;
mod runtime;
mod terminal;
mod thread_lifecycle;
mod worktree_inventory;
mod worktrees;

pub(crate) use worktrees::{
    collect_filter_override_names, collect_local_filter_overrides,
    prepare_execution_root_with_merge_target, repository_identity, CodePinnedReadCommand,
};
#[cfg(unix)]
pub(crate) use worktrees::{spawn_pinned_read_git_helper, PinnedGitChild};

pub use approvals::CodeApprovalResponseInput;
pub(crate) use bindings::removal::{
    recover_pending_worktree_removals, remove_archived_worktree, CodeWorktreeRemovalContext,
};
pub use bindings::removal::{CodeWorktreeRemovalReceipt, CodeWorktreeRemoveInput};
pub use bindings::{
    CodeExecutionMode, CodeThreadBinding, CodeThreadBindingLookupInput, CodeThreadBindingScope,
    CodeThreadBindingStore, CodeThreadLifecycleStatus, CodeThreadPreparation,
    CodeThreadPreparationOperation,
};
pub(crate) use bindings::{CodeThreadBindingLifecycle, CodeThreadLifecycleClaim};
pub use discovery::CodeRuntimeProbe;
pub(crate) use git_write::CodeGitWriteState;
pub use git_write::{
    CodeGitAcknowledgeInput, CodeGitAcknowledgeReceipt, CodeGitChangeFile, CodeGitChangeSet,
    CodeGitChangeStatus, CodeGitCommitInput, CodeGitCommitReceipt, CodeGitIndexMutationInput,
    CodeGitIndexMutationReceipt, CodeGitReconcileInput, CodeGitReconcileResult, CodeGitStatus,
    CodeGitStatusInput,
};
pub(crate) use model_catalog::CodeModelSelectionStore;
// These nested DTOs are part of the serialized `CodeModelsListResult` contract
// even though production command code names only the enclosing result type.
#[allow(unused_imports)]
pub use model_catalog::{CodeModelOption, CodeReasoningEffortOption};
pub use model_catalog::{CodeModelSelection, CodeModelsListResult};
pub(crate) use paths::canonical_workspace_root;
pub(crate) use protocol::{
    code_thread_source, CodeRecoveryThread, CodeRuntimeEvent, CodeRuntimeEventBacklog,
    CodeThreadRpcOpenResult,
};
pub use protocol::{
    CodeActiveTurnCheckpoint, CodeApprovalCheckpoint, CodeBoundThreadOpenResult,
    CodeBoundThreadSummary, CodeEventBacklog, CodeEventCheckpoint, CodePreparedWorktree,
    CodeThreadBindingRecoverInput, CodeThreadChangeStatus, CodeThreadChangedFile,
    CodeThreadChanges, CodeThreadChangesInput, CodeThreadForkInput,
    CodeThreadLifecycleMutationResult, CodeThreadListInput, CodeThreadPreparationListInput,
    CodeThreadRenameInput, CodeThreadResumeInput, CodeThreadStartError, CodeThreadStartInput,
    CodeThreadSummary, CodeThreadsPage, CodeTurnInterruptInput, CodeTurnStartInput,
    CodeTurnSteerInput, CodeTurnSummary, CodeWorkspaceEvent, CodeWorktreePrepareCommandInput,
};
#[cfg(test)]
pub(crate) use protocol::{
    CodeRuntimeActiveTurnCheckpoint, CodeRuntimeApprovalCheckpoint, CodeRuntimeEventCheckpoint,
};
pub(crate) use runtime::{
    CodeEventEmitter, CodeRpcDeliveryError, CodeThreadLifecycleDirtyCheckpoint,
    CodeThreadLifecycleGraphProof,
};
pub use runtime::{CodeRuntime, CodeRuntimeStatus, CODE_WORKSPACE_EVENT};
pub use terminal::{
    CodeTerminalEvent, CodeTerminalManager, CodeTerminalOpenInput, CodeTerminalResizeInput,
    CodeTerminalSession, CodeTerminalStdinInput, CodeTerminalTerminateInput,
};
pub use thread_lifecycle::CodeThreadLifecycleInput;
pub(crate) use thread_lifecycle::{
    CodeAuthoritativeThreadGraph, CodePendingForkExpectation, CodeThreadMembership,
};
pub use worktree_inventory::{
    list_worktree_inventory, CodeWorktreeInventoryRow, CodeWorktreesListInput,
};
pub use worktrees::{
    preflight_execution_root, prepare_execution_root, revalidate_execution_root,
    CodeRepositoryDescriptor, CodeRepositoryInspectInput, CodeWorktreeDescriptor,
    CodeWorktreePrepareInput, CodeWorktreeStatus,
};
