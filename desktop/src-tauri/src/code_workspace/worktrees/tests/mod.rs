use super::{
    execution_root::*, git::*, pinned_command::*, pinned_operation::*, pinned_verify::*,
    process::*, repository::*, *,
};

mod execution_root;
mod merge_proof;
mod pinned_security;
mod repository;
mod support;

use support::*;

#[cfg(unix)]
#[test]
#[ignore = "private subprocess entry exercised by managed-worktree tests"]
fn pinned_git_helper_subprocess_entry() {
    if std::env::var_os(PINNED_GIT_REQUEST_ENV).is_none() {
        return;
    }
    if let Err(error) = execute_pinned_git_helper() {
        panic!("{error}");
    }
}
