mod engine;
mod git_command;
mod journal;
mod owned_lock;
mod private_artifact;
mod protocol;
mod repository;
mod startup;
#[cfg(test)]
mod tests;
mod transaction;

pub(crate) use engine::CodeGitWriteState;
pub(crate) use engine::{
    acknowledge, blocked_status, commit, ensure_admission_clear, reconcile,
    recovery_required_status, stage, status, unstage, GitWriteContext,
};
#[cfg(target_os = "macos")]
pub(crate) use git_command::macos_root_trusted_git;
#[cfg(target_os = "macos")]
pub(in crate::code_workspace) use git_command::prepare_macos_git_write;
#[cfg(target_os = "linux")]
pub(in crate::code_workspace) use git_command::RootTrustedGit;
pub use protocol::*;
pub(crate) use startup::recover_startup_journals;

pub(super) fn with_git_authority<T>(
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        crate::code_workspace::macos_git_xpc::with_authority_session(|_| operation())
    }
    #[cfg(any(not(target_os = "macos"), test))]
    {
        operation()
    }
}
