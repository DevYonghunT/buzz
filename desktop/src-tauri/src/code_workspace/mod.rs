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
mod jsonrpc;
mod paths;
mod protocol;
mod runtime;
mod worktrees;

#[cfg(unix)]
pub(crate) use worktrees::run_pinned_git_helper_if_requested;

pub use approvals::CodeApprovalResponseInput;
pub use bindings::{
    CodeThreadBinding, CodeThreadBindingLookupInput, CodeThreadBindingScope,
    CodeThreadBindingStore, CodeThreadPreparation,
};
pub use discovery::CodeRuntimeProbe;
pub(crate) use paths::canonical_workspace_root;
#[cfg(test)]
pub(crate) use protocol::CodeThreadSummary;
pub(crate) use protocol::{
    code_thread_source, CodeRecoveryThread, CodeRuntimeEvent, CodeRuntimeEventBacklog,
};
pub use protocol::{
    CodeBoundThreadOpenResult, CodeBoundThreadSummary, CodeEventBacklog, CodePreparedWorktree,
    CodeThreadBindingRecoverInput, CodeThreadListInput, CodeThreadPreparationListInput,
    CodeThreadResumeInput, CodeThreadStartError, CodeThreadStartInput, CodeThreadsPage,
    CodeTurnInterruptInput, CodeTurnStartInput, CodeTurnSteerInput, CodeTurnSummary,
    CodeWorkspaceEvent, CodeWorktreePrepareCommandInput,
};
pub use runtime::{CodeRuntime, CodeRuntimeStatus, CODE_WORKSPACE_EVENT};
pub use worktrees::{
    preflight_execution_root, prepare_execution_root, revalidate_execution_root,
    CodeRepositoryDescriptor, CodeRepositoryInspectInput, CodeWorktreeDescriptor,
    CodeWorktreeStatus,
};
