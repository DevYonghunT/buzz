use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
#[cfg(any(not(target_os = "macos"), test))]
use std::process::Child;
use std::process::{Command, ExitStatus, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::bindings::{
    validate_direct_local_branch_ref, CodeExecutionMode, CodeThreadBindingLookupInput,
    CodeThreadBindingStore,
};
#[cfg(target_os = "linux")]
use super::git_launch::GitLaunchAuthority;
#[cfg(target_os = "macos")]
use super::macos_git_xpc::{self, DescriptorObservation, MacGitProcessSpec};
#[cfg(all(target_os = "macos", not(test)))]
use super::macos_git_xpc::{MacGitAuthoritySession, MacGitChild, MacGitFamily, MacGitInput};

const WORKTREES_DIRECTORY: &str = "WORKTREES";
const GIT_TIMEOUT: Duration = Duration::from_secs(60);
const GIT_OUTPUT_LIMIT: usize = 1024 * 1024;
const MAX_BASE_REF_BYTES: usize = 4 * 1024;
const WORKTREE_ID_ATTEMPTS: usize = 16;
#[cfg(target_os = "macos")]
const MACOS_SYSTEM_GIT: &str = "/usr/bin/git";
#[cfg(all(unix, test))]
const PINNED_GIT_REQUEST_ENV: &str = "SCHOOLX_CODE_PINNED_GIT_REQUEST_V1";
#[cfg(unix)]
const PINNED_GIT_REQUEST_VERSION: u32 = 1;
#[cfg(unix)]
const MAX_PINNED_GIT_REQUEST_BYTES: usize = 64 * 1024;
#[cfg(unix)]
const MAX_PINNED_GIT_FILTER_KEYS: usize = 128;
#[cfg(unix)]
const MAX_PINNED_GIT_PATH_BYTES: usize = 16 * 1024;

#[cfg(unix)]
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "camelCase", deny_unknown_fields)]
enum PinnedGitRequest {
    WorktreeAdd {
        git_executable: String,
        git_common_dir: String,
        base_commit: String,
        disabled_filter_keys: Vec<String>,
        expected_target_path: String,
    },
    Checkout {
        git_executable: String,
        base_commit: String,
        disabled_filter_keys: Vec<String>,
        expected_target_path: String,
    },
    ReadOnly {
        git_executable: String,
        command: CodePinnedReadCommand,
        disabled_filter_keys: Vec<String>,
        expected_target_path: String,
    },
}

/// Closed set of Git reads allowed through the pinned-directory helper.
/// Keeping paths and commits typed prevents a read request from smuggling a
/// mutating Git option such as `diff --output` into the helper.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum CodePinnedReadCommand {
    TopLevel,
    CommonDir,
    LocalConfig,
    WorktreeConfigNames,
    ResolveCommit {
        base_ref: String,
    },
    VerifyCommit {
        commit: String,
    },
    HeadCommit,
    CurrentBranch,
    StatusPorcelain,
    DirectLocalRefCommit {
        target_ref: String,
    },
    MergeBaseIsAncestor {
        head_commit: String,
        target_commit: String,
    },
    TrackedNumstat {
        base_commit: String,
    },
    TrackedNameStatus {
        base_commit: String,
    },
    TrackedUnmergedPaths,
    TrackedPatch {
        base_commit: String,
        path: String,
    },
    UntrackedPaths,
    UntrackedPatch {
        path: String,
    },
}

#[cfg(unix)]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PinnedGitEnvelope {
    version: u32,
    target_device: u64,
    target_inode: u64,
    request: PinnedGitRequest,
}

#[cfg(all(target_os = "macos", not(test)))]
/// Platform child returned by the signed macOS Git service.
pub(crate) type PinnedGitChild = MacGitChild;
#[cfg(any(not(target_os = "macos"), test))]
/// Platform child returned by Linux direct launch or the test crash harness.
pub(crate) type PinnedGitChild = Child;

#[cfg(unix)]
impl PinnedGitRequest {
    fn expected_target_path(&self) -> &Path {
        let path = match self {
            Self::WorktreeAdd {
                expected_target_path,
                ..
            }
            | Self::Checkout {
                expected_target_path,
                ..
            }
            | Self::ReadOnly {
                expected_target_path,
                ..
            } => expected_target_path,
        };
        Path::new(path)
    }

    fn git_executable(&self) -> &Path {
        let path = match self {
            Self::WorktreeAdd { git_executable, .. }
            | Self::Checkout { git_executable, .. }
            | Self::ReadOnly { git_executable, .. } => git_executable,
        };
        Path::new(path)
    }
}

#[cfg(unix)]
#[derive(Clone)]
struct PinnedGitLaunchAuthority {
    #[cfg(target_os = "linux")]
    direct: GitLaunchAuthority,
    #[cfg(all(target_os = "macos", not(test)))]
    session: MacGitAuthoritySession,
}

#[cfg(unix)]
impl PinnedGitLaunchAuthority {
    fn admit(probe_directory: &fs::File) -> Result<Self, String> {
        #[cfg(target_os = "linux")]
        {
            GitLaunchAuthority::admit(probe_directory).map(|direct| Self { direct })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = probe_directory;
            #[cfg(all(target_os = "macos", not(test)))]
            {
                drop(super::git_write::macos_root_trusted_git()?);
                let session = MacGitAuthoritySession::begin()?;
                Ok(Self { session })
            }
            #[cfg(all(not(test), not(target_os = "macos")))]
            {
                Err("pinned Git launch is unsupported on this Unix platform".to_string())
            }
            #[cfg(test)]
            {
                Ok(Self {})
            }
        }
    }

    fn git_executable(&self) -> Result<PathBuf, String> {
        #[cfg(target_os = "linux")]
        {
            Ok(self.direct.path().to_path_buf())
        }
        #[cfg(target_os = "macos")]
        {
            Ok(PathBuf::from(MACOS_SYSTEM_GIT))
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            Err("pinned Git launch is unsupported on this Unix platform".to_string())
        }
    }

    fn bind_request(&self, request: &mut PinnedGitRequest) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            let executable = path_to_string(self.direct.path(), "root-trusted Git executable")?;
            match request {
                PinnedGitRequest::WorktreeAdd { git_executable, .. }
                | PinnedGitRequest::Checkout { git_executable, .. }
                | PinnedGitRequest::ReadOnly { git_executable, .. } => {
                    *git_executable = executable;
                }
            }
        }
        #[cfg(target_os = "macos")]
        {
            let executable = MACOS_SYSTEM_GIT.to_string();
            match request {
                PinnedGitRequest::WorktreeAdd { git_executable, .. }
                | PinnedGitRequest::Checkout { git_executable, .. }
                | PinnedGitRequest::ReadOnly { git_executable, .. } => {
                    *git_executable = executable;
                }
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        let _ = request;
        Ok(())
    }

    fn spawn(
        &self,
        request: &PinnedGitRequest,
        target: &fs::File,
    ) -> Result<PinnedGitChild, String> {
        #[cfg(target_os = "linux")]
        {
            spawn_pinned_git_direct_child(request, target, &self.direct)
        }
        #[cfg(all(target_os = "macos", not(test)))]
        {
            spawn_pinned_git_macos_child(request, target, &self.session)
        }
        #[cfg(all(not(target_os = "linux"), test))]
        {
            spawn_pinned_git_path_helper_child(request, target)
        }
        #[cfg(all(not(target_os = "linux"), not(target_os = "macos"), not(test)))]
        {
            let _ = (request, target);
            Err("pinned Git launch is unsupported on this Unix platform".to_string())
        }
    }
}

#[cfg(unix)]
fn with_pinned_git_authority<T>(
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        macos_git_xpc::with_authority_session(|_| operation())
    }
    #[cfg(any(target_os = "linux", test))]
    {
        operation()
    }
    #[cfg(all(not(test), not(any(target_os = "linux", target_os = "macos"))))]
    Err("pinned Git launch is unsupported on this Unix platform".to_string())
}

/// Batch a caller's related execution-root reads under one authenticated Git
/// authority. Nested pinned reads reuse the same same-thread macOS XPC session,
/// while Linux and test builds preserve their existing descriptor-bound path.
pub(crate) fn with_execution_root_authority<T>(
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    #[cfg(unix)]
    {
        with_pinned_git_authority(operation)
    }
    #[cfg(not(unix))]
    operation()
}

/// Native input for resolving a repository and preparing an execution root.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeWorktreePrepareInput {
    /// Any existing directory inside the selected Git worktree.
    pub repository_root: String,
    /// Git revision to resolve to an immutable commit before preparation.
    pub base_ref: String,
    /// Whether preparation should create a managed worktree or stay local.
    pub execution_mode: CodeExecutionMode,
}

/// Native input for inspecting a repository without preparing an execution root.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeRepositoryInspectInput {
    /// Any existing directory inside the selected Git worktree.
    pub repository_root: String,
    /// Git revision that must resolve before the repository is accepted.
    pub base_ref: String,
}

/// Canonical identity of a local Git repository.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeRepositoryDescriptor {
    /// Canonical top-level directory of the selected Git worktree.
    pub repository_root: String,
    /// Canonical Git common directory shared by all linked worktrees.
    pub git_common_dir: String,
    /// Lowercase domain-separated SHA-256 identity of the canonical Git
    /// common-directory path.
    pub repository_identity: String,
}

/// Persistable native descriptor for one prepared execution root.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeWorktreeDescriptor {
    /// Whether the execution root is managed or is the selected checkout.
    pub execution_mode: CodeExecutionMode,
    /// Canonical identity shared by every linked worktree of the repository.
    pub repository_identity: String,
    /// Canonical directory that may be passed to Codex as `cwd`.
    pub execution_root: String,
    /// Immutable commit resolved from the requested base ref.
    pub base_ref: String,
    /// UUID directory name for managed worktrees; absent in local mode.
    pub worktree_id: Option<String>,
}

/// Result of preparing a local or managed SchoolX Code execution root.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeWorktreePrepareResult {
    /// Canonical source repository metadata discovered during preparation.
    pub repository: CodeRepositoryDescriptor,
    /// Descriptor suitable for the durable thread-binding index.
    pub descriptor: CodeWorktreeDescriptor,
    /// Current commit checked out at the prepared execution root.
    pub head_commit: String,
    /// Current branch name, or `None` for a detached `HEAD`.
    pub branch: Option<String>,
    /// Whether tracked or untracked files currently differ from `HEAD`.
    pub dirty: bool,
}

/// Native preparation result carrying optional merge authority outside the
/// public Tauri response shape.
pub(crate) struct CodePreparedExecutionRoot {
    pub(crate) worktree: CodeWorktreePrepareResult,
    pub(crate) merge_target_ref: Option<String>,
}

/// Revalidated status of a previously persisted execution-root descriptor.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeWorktreeStatus {
    /// Canonical descriptor after all filesystem and Git checks complete.
    pub descriptor: CodeWorktreeDescriptor,
    /// Current commit checked out at the execution root.
    pub head_commit: String,
    /// Current branch name, or `None` for a detached `HEAD`.
    pub branch: Option<String>,
    /// Whether tracked or untracked files currently differ from `HEAD`.
    pub dirty: bool,
}

/// Immutable evidence returned only when the native-authorized local branch
/// contains the exact managed-worktree HEAD in Git's object graph.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(dead_code)]
pub(crate) struct CodeMergeProofReceipt {
    pub(crate) repository_identity: String,
    pub(crate) worktree_id: String,
    pub(crate) head_commit: String,
    pub(crate) target_ref: String,
    pub(crate) target_commit: String,
}

/// Closed result of a successful, stable ancestry inspection.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) enum CodeMergeProofOutcome {
    Proven(CodeMergeProofReceipt),
    NotMerged,
}

#[cfg(unix)]
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
struct CodeMergeProofPathIdentity {
    path: PathBuf,
    device: u64,
    inode: u64,
}

#[cfg(unix)]
#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
struct CodeMergeProofSnapshot {
    head_commit: String,
    target_commit: String,
    root: CodeMergeProofPathIdentity,
    common_dir: CodeMergeProofPathIdentity,
}

/// Opaque native request and pinned directories for one managed Git mutation.
///
/// This value is produced only after every named path has been validated. It
/// retains open handles for the whole managed-directory chain and revalidates
/// every named component immediately before Git is spawned.
#[cfg(unix)]
pub(crate) struct CodePinnedGitOperation {
    request: PinnedGitRequest,
    directories: Vec<fs::File>,
    launch: PinnedGitLaunchAuthority,
}

#[cfg(unix)]
impl CodePinnedGitOperation {
    /// Execute through the private helper using the pinned target handle.
    pub(crate) fn execute(self) -> Result<Vec<u8>, String> {
        verify_pinned_target_chain(&self.request, &self.directories)?;
        let output = spawn_pinned_git_helper(&self.request, &self.directories, &self.launch)?;
        verify_pinned_target_chain(&self.request, &self.directories)?;
        Ok(output)
    }
}

#[derive(Debug)]
struct RepositoryInfo {
    top_level: PathBuf,
    common_dir: PathBuf,
    identity: String,
}

#[derive(Clone, Copy)]
enum GitOperation {
    ReadOnly,
    WorkingTreeRead,
    Mutating,
}

impl GitOperation {
    #[cfg(not(unix))]
    fn may_run_repository_filters(self) -> bool {
        matches!(self, Self::WorkingTreeRead | Self::Mutating)
    }
}

struct CapturedPipe {
    bytes: Vec<u8>,
    truncated: bool,
}

struct CapturedChild {
    status: ExitStatus,
    stdout: CapturedPipe,
    stderr: CapturedPipe,
}

mod execution_root;
mod git;
mod pinned_command;
mod pinned_operation;
mod pinned_verify;
mod process;
mod repository;

pub use execution_root::{
    preflight_execution_root, prepare_execution_root, revalidate_execution_root,
};
pub(crate) use execution_root::{
    prepare_execution_root_with_merge_target, prove_binding_merge_target_before,
    prove_direct_local_ancestry_before, revalidate_execution_root_before,
};
pub(crate) use git::{collect_filter_override_names, collect_local_filter_overrides};
#[cfg(target_os = "linux")]
use pinned_command::spawn_pinned_git_direct_child;
#[cfg(unix)]
use pinned_command::spawn_pinned_git_helper;
#[cfg(all(target_os = "macos", not(test)))]
use pinned_command::spawn_pinned_git_macos_child;
#[cfg(all(not(target_os = "linux"), test))]
use pinned_command::spawn_pinned_git_path_helper_child;
#[cfg(unix)]
pub(crate) use pinned_command::{
    spawn_pinned_read_git_helper, strip_linux_onnxruntime_startup_diagnostic,
};
#[cfg(target_os = "macos")]
pub(in crate::code_workspace) use pinned_operation::prepare_macos_pinned_git;
#[cfg(unix)]
use pinned_verify::verify_pinned_target_chain;
#[cfg(target_os = "linux")]
use repository::path_to_string;
pub(crate) use repository::repository_identity;

#[cfg(test)]
mod tests;
