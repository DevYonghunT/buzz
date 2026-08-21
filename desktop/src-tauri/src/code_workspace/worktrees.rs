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
                macos_git_xpc::require_capability()?;
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

#[cfg(test)]
fn inspect_repository(repository_root: &str) -> Result<CodeRepositoryDescriptor, String> {
    let repository = discover_repository(Path::new(repository_root))?;
    repository_descriptor(&repository)
}

/// Resolve `base_ref` and prepare an execution root below the supplied active
/// SchoolX nest. Local mode deliberately does not inspect or create the nest.
pub fn prepare_execution_root(
    input: CodeWorktreePrepareInput,
    nest_root: &Path,
) -> Result<CodeWorktreePrepareResult, String> {
    prepare_execution_root_with_merge_target(input, nest_root).map(|prepared| prepared.worktree)
}

/// Prepare an execution root while capturing optional direct-local merge
/// authority before the first managed-worktree mutation.
pub(crate) fn prepare_execution_root_with_merge_target(
    input: CodeWorktreePrepareInput,
    nest_root: &Path,
) -> Result<CodePreparedExecutionRoot, String> {
    #[cfg(unix)]
    {
        with_pinned_git_authority(|| {
            prepare_execution_root_with_merge_target_inner(input, nest_root)
        })
    }
    #[cfg(not(unix))]
    prepare_execution_root_with_merge_target_inner(input, nest_root)
}

fn prepare_execution_root_with_merge_target_inner(
    input: CodeWorktreePrepareInput,
    nest_root: &Path,
) -> Result<CodePreparedExecutionRoot, String> {
    #[cfg(not(unix))]
    if input.execution_mode == CodeExecutionMode::Worktree {
        return Err(
            "SchoolX Code managed worktree launch is unsupported on this platform".to_string(),
        );
    }

    let repository = discover_repository(Path::new(&input.repository_root))?;
    let repository_descriptor = repository_descriptor(&repository)?;
    let base_commit = resolve_commit(&repository.top_level, &input.base_ref)?;
    let merge_target_ref = if input.execution_mode == CodeExecutionMode::Worktree {
        capture_direct_local_merge_target(&repository, &input.base_ref, &base_commit)?
    } else {
        None
    };
    let status = match input.execution_mode {
        CodeExecutionMode::Local => {
            let descriptor = CodeWorktreeDescriptor {
                execution_mode: CodeExecutionMode::Local,
                repository_identity: repository.identity.clone(),
                execution_root: path_to_string(&repository.top_level, "execution root")?,
                base_ref: base_commit,
                worktree_id: None,
            };
            revalidate_execution_root(&descriptor, nest_root)?
        }
        CodeExecutionMode::Worktree => {
            #[cfg(unix)]
            {
                let probe_directory = pin_git_directory(&repository.top_level)?;
                let launch = PinnedGitLaunchAuthority::admit(&probe_directory)?;
                let descriptor =
                    prepare_managed_worktree(&repository, &base_commit, nest_root, &launch)?;
                revalidate_execution_root(&descriptor, nest_root)?
            }
            #[cfg(not(unix))]
            {
                return Err(
                    "SchoolX Code managed worktree launch is unsupported on this platform"
                        .to_string(),
                );
            }
        }
    };

    Ok(CodePreparedExecutionRoot {
        worktree: CodeWorktreePrepareResult {
            repository: repository_descriptor,
            descriptor: status.descriptor,
            head_commit: status.head_commit,
            branch: status.branch,
            dirty: status.dirty,
        },
        merge_target_ref,
    })
}

fn capture_direct_local_merge_target(
    repository: &RepositoryInfo,
    selected_base: &str,
    resolved_base: &str,
) -> Result<Option<String>, String> {
    validate_commit_id(resolved_base)?;
    let selected_base = validate_base_ref(selected_base)?;
    let target_ref = if selected_base == "HEAD" {
        let Some(branch) = repository_branch_until(&repository.top_level, None)? else {
            return Ok(None);
        };
        format!("refs/heads/{branch}")
    } else if selected_base.starts_with("refs/heads/") {
        selected_base.to_string()
    } else {
        if selected_base.starts_with("refs/") || validate_commit_id(selected_base).is_ok() {
            return Ok(None);
        }
        format!("refs/heads/{selected_base}")
    };
    if validate_direct_local_branch_ref(&target_ref).is_err() {
        return Ok(None);
    }
    let Ok(target_commit) = resolve_commit(&repository.top_level, &target_ref) else {
        return Ok(None);
    };
    if target_commit == resolved_base {
        Ok(Some(target_ref))
    } else {
        Ok(None)
    }
}

/// Inspect a selected checkout and resolve its base ref without creating a
/// managed worktree. The command facade uses this preflight to validate the
/// caller's repository scope before any Git mutation.
pub fn preflight_execution_root(
    repository_root: &str,
    base_ref: &str,
) -> Result<CodeRepositoryDescriptor, String> {
    #[cfg(unix)]
    {
        with_pinned_git_authority(|| preflight_execution_root_inner(repository_root, base_ref))
    }
    #[cfg(not(unix))]
    preflight_execution_root_inner(repository_root, base_ref)
}

fn preflight_execution_root_inner(
    repository_root: &str,
    base_ref: &str,
) -> Result<CodeRepositoryDescriptor, String> {
    let repository = discover_repository(Path::new(repository_root))?;
    resolve_commit(&repository.top_level, base_ref)?;
    repository_descriptor(&repository)
}

/// Revalidate a persisted descriptor against the current filesystem, nest
/// boundary, Git common directory, and repository identity before reuse.
pub fn revalidate_execution_root(
    descriptor: &CodeWorktreeDescriptor,
    nest_root: &Path,
) -> Result<CodeWorktreeStatus, String> {
    revalidate_execution_root_with_authority(descriptor, nest_root, None)
}

/// Revalidate a persisted descriptor while bounding all Git subprocesses by
/// one caller-owned deadline.
pub(crate) fn revalidate_execution_root_before(
    descriptor: &CodeWorktreeDescriptor,
    nest_root: &Path,
    deadline: Instant,
) -> Result<CodeWorktreeStatus, String> {
    revalidate_execution_root_with_authority(descriptor, nest_root, Some(deadline))
}

fn revalidate_execution_root_with_authority(
    descriptor: &CodeWorktreeDescriptor,
    nest_root: &Path,
    deadline: Option<Instant>,
) -> Result<CodeWorktreeStatus, String> {
    #[cfg(unix)]
    {
        with_pinned_git_authority(|| {
            revalidate_execution_root_until(descriptor, nest_root, deadline)
        })
    }
    #[cfg(not(unix))]
    revalidate_execution_root_until(descriptor, nest_root, deadline)
}

fn revalidate_execution_root_until(
    descriptor: &CodeWorktreeDescriptor,
    nest_root: &Path,
    deadline: Option<Instant>,
) -> Result<CodeWorktreeStatus, String> {
    validate_repository_identity(&descriptor.repository_identity)?;
    validate_commit_id(&descriptor.base_ref)?;

    let execution_root = match descriptor.execution_mode {
        CodeExecutionMode::Local => {
            if descriptor.worktree_id.is_some() {
                return Err("local SchoolX Code execution cannot have a worktree id".to_string());
            }
            let execution_root = canonical_existing_directory(
                Path::new(&descriptor.execution_root),
                "SchoolX Code local execution root",
            )?;
            execution_root
        }
        CodeExecutionMode::Worktree => {
            let worktree_id = descriptor.worktree_id.as_deref().ok_or_else(|| {
                "managed SchoolX Code execution requires a worktree id".to_string()
            })?;
            validate_worktree_id(worktree_id)?;
            validate_managed_execution_root(
                nest_root,
                &descriptor.repository_identity,
                worktree_id,
                Path::new(&descriptor.execution_root),
            )?
        }
    };

    let execution_repository = discover_repository_until(&execution_root, deadline)?;
    if execution_repository.top_level != execution_root {
        return Err("SchoolX Code execution root is not a Git top-level directory".to_string());
    }
    if execution_repository.identity != descriptor.repository_identity {
        return Err("SchoolX Code execution root belongs to a different repository".to_string());
    }
    let resolved_base = resolve_commit_until(&execution_root, &descriptor.base_ref, deadline)?;
    if resolved_base != descriptor.base_ref {
        return Err("stored SchoolX Code base commit changed".to_string());
    }

    let canonical_execution = path_to_string(&execution_root, "execution root")?;
    if canonical_execution != descriptor.execution_root {
        return Err("stored SchoolX Code execution root is not canonical".to_string());
    }
    let head_commit = resolve_commit_until(&execution_root, "HEAD", deadline)?;
    let branch = repository_branch_until(&execution_root, deadline)?;
    let dirty = repository_is_dirty_until(&execution_root, deadline)?;

    Ok(CodeWorktreeStatus {
        descriptor: descriptor.clone(),
        head_commit,
        branch,
        dirty,
    })
}

/// Prove ancestry only from one exact binding-store snapshot. Missing native
/// authority remains a closed `None`; no ref is inferred from `base_ref`.
#[allow(dead_code)]
pub(crate) fn prove_binding_merge_target_before(
    store: &CodeThreadBindingStore,
    input: &CodeThreadBindingLookupInput,
    nest_root: &Path,
    deadline: Instant,
) -> Result<Option<CodeMergeProofOutcome>, String> {
    let Some((binding, target_ref)) = store.binding_merge_authority(input)? else {
        return Ok(None);
    };
    let Some(target_ref) = target_ref else {
        return Ok(None);
    };
    let descriptor = CodeWorktreeDescriptor {
        execution_mode: binding.execution_mode,
        repository_identity: binding.repository_identity,
        execution_root: binding.execution_root,
        base_ref: binding.base_ref,
        worktree_id: binding.worktree_id,
    };
    #[cfg(all(target_os = "macos", not(test)))]
    {
        with_pinned_git_authority(|| {
            prove_direct_local_ancestry_before(&descriptor, nest_root, &target_ref, deadline)
                .map(Some)
        })
    }
    #[cfg(any(not(target_os = "macos"), test))]
    prove_direct_local_ancestry_before(&descriptor, nest_root, &target_ref, deadline).map(Some)
}

/// Run one bounded, read-only graph proof against a persisted direct local ref.
/// Exit 0 from `merge-base --is-ancestor` is the only positive result; exit 1
/// is a stable negative and every other condition is unavailable/error.
#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn prove_direct_local_ancestry_before(
    descriptor: &CodeWorktreeDescriptor,
    nest_root: &Path,
    target_ref: &str,
    deadline: Instant,
) -> Result<CodeMergeProofOutcome, String> {
    prove_direct_local_ancestry_with_hook(descriptor, nest_root, target_ref, deadline, || Ok(()))
}

#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
fn prove_direct_local_ancestry_with_hook<F>(
    descriptor: &CodeWorktreeDescriptor,
    nest_root: &Path,
    target_ref: &str,
    deadline: Instant,
    after_ancestry: F,
) -> Result<CodeMergeProofOutcome, String>
where
    F: FnOnce() -> Result<(), String>,
{
    with_pinned_git_authority(|| {
        prove_direct_local_ancestry_with_hook_inner(
            descriptor,
            nest_root,
            target_ref,
            deadline,
            after_ancestry,
        )
    })
}

#[cfg(unix)]
fn prove_direct_local_ancestry_with_hook_inner<F>(
    descriptor: &CodeWorktreeDescriptor,
    nest_root: &Path,
    target_ref: &str,
    deadline: Instant,
    after_ancestry: F,
) -> Result<CodeMergeProofOutcome, String>
where
    F: FnOnce() -> Result<(), String>,
{
    validate_direct_local_branch_ref(target_ref)?;
    if descriptor.execution_mode != CodeExecutionMode::Worktree {
        return Err("SchoolX Code merge proof requires a managed worktree".to_string());
    }
    let worktree_id = descriptor
        .worktree_id
        .as_deref()
        .ok_or_else(|| "SchoolX Code merge proof is missing its worktree id".to_string())?;
    validate_worktree_id(worktree_id)?;

    let first = read_merge_proof_snapshot(descriptor, nest_root, target_ref, deadline)?;
    let is_ancestor = run_pinned_ancestry_before(
        Path::new(&descriptor.execution_root),
        &first.head_commit,
        &first.target_commit,
        deadline,
    )?;
    after_ancestry()?;
    let second = read_merge_proof_snapshot(descriptor, nest_root, target_ref, deadline)?;
    if first != second {
        return Err("SchoolX Code merge proof inputs changed during inspection".to_string());
    }
    if !is_ancestor {
        return Ok(CodeMergeProofOutcome::NotMerged);
    }
    Ok(CodeMergeProofOutcome::Proven(CodeMergeProofReceipt {
        repository_identity: descriptor.repository_identity.clone(),
        worktree_id: worktree_id.to_string(),
        head_commit: first.head_commit,
        target_ref: target_ref.to_string(),
        target_commit: first.target_commit,
    }))
}

#[cfg(not(unix))]
#[allow(dead_code)]
pub(crate) fn prove_direct_local_ancestry_before(
    _descriptor: &CodeWorktreeDescriptor,
    _nest_root: &Path,
    _target_ref: &str,
    _deadline: Instant,
) -> Result<CodeMergeProofOutcome, String> {
    Err("SchoolX Code pinned merge proof is unsupported on this platform".to_string())
}

#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
fn read_merge_proof_snapshot(
    descriptor: &CodeWorktreeDescriptor,
    nest_root: &Path,
    target_ref: &str,
    deadline: Instant,
) -> Result<CodeMergeProofSnapshot, String> {
    if Instant::now() >= deadline {
        return Err("SchoolX Code merge proof budget was exhausted".to_string());
    }
    let status = revalidate_execution_root_before(descriptor, nest_root, deadline)?;
    let execution_root = Path::new(&descriptor.execution_root);
    let repository = discover_repository_until(execution_root, Some(deadline))?;
    if repository.top_level != execution_root
        || repository.identity != descriptor.repository_identity
    {
        return Err("SchoolX Code merge proof repository identity changed".to_string());
    }
    reject_legacy_grafts(&repository.common_dir)?;
    let root = merge_proof_path_identity(execution_root, "managed worktree root")?;
    let common_dir = merge_proof_path_identity(&repository.common_dir, "Git common directory")?;
    let head_output =
        run_pinned_read_before(execution_root, CodePinnedReadCommand::HeadCommit, deadline)?;
    let head_commit = single_text_output(&head_output, "merge-proof HEAD commit")?;
    validate_commit_id(&head_commit)?;
    if head_commit != status.head_commit {
        return Err("SchoolX Code merge-proof HEAD changed during snapshot".to_string());
    }
    let target_output = run_pinned_read_before(
        execution_root,
        CodePinnedReadCommand::DirectLocalRefCommit {
            target_ref: target_ref.to_string(),
        },
        deadline,
    )?;
    let target_commit = single_text_output(&target_output, "merge-proof target commit")?;
    validate_commit_id(&target_commit)?;
    reject_legacy_grafts(&repository.common_dir)?;
    Ok(CodeMergeProofSnapshot {
        head_commit,
        target_commit,
        root,
        common_dir,
    })
}

#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
fn merge_proof_path_identity(
    path: &Path,
    label: &str,
) -> Result<CodeMergeProofPathIdentity, String> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = path
        .symlink_metadata()
        .map_err(|error| format!("failed to inspect merge-proof {label}: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "SchoolX Code merge-proof {label} is not a real directory"
        ));
    }
    Ok(CodeMergeProofPathIdentity {
        path: path.to_path_buf(),
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
fn reject_legacy_grafts(common_dir: &Path) -> Result<(), String> {
    let info = common_dir.join("info");
    let info_metadata = match info.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("failed to inspect Git info directory: {error}")),
    };
    if info_metadata.file_type().is_symlink() || !info_metadata.is_dir() {
        return Err("SchoolX Code merge proof rejected an unsafe Git info directory".to_string());
    }
    let grafts = info.join("grafts");
    let graft_metadata = match grafts.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("failed to inspect legacy Git grafts: {error}")),
    };
    if graft_metadata.file_type().is_symlink() || !graft_metadata.is_file() {
        return Err("SchoolX Code merge proof rejected an unsafe legacy graft file".to_string());
    }
    if graft_metadata.len() != 0 {
        return Err("SchoolX Code merge proof rejected non-empty legacy Git grafts".to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn prepare_managed_worktree(
    repository: &RepositoryInfo,
    base_commit: &str,
    nest_root: &Path,
    launch: &PinnedGitLaunchAuthority,
) -> Result<CodeWorktreeDescriptor, String> {
    let nest_root = canonical_existing_directory(nest_root, "SchoolX nest root")?;
    let worktrees_root = ensure_real_child_directory(&nest_root, WORKTREES_DIRECTORY)?;
    let repository_root = ensure_real_child_directory(&worktrees_root, &repository.identity)?;

    let (worktree_id, target) = reserve_worktree_target(&repository_root)?;
    validate_reserved_worktree_target(&repository_root, &target)?;
    add_managed_worktree(repository, &target, base_commit, launch)?;

    let execution_root =
        validate_managed_execution_root(&nest_root, &repository.identity, &worktree_id, &target)?;
    set_owner_only_directory(&execution_root)?;
    let created_repository = discover_repository(&execution_root)?;
    if created_repository.top_level != execution_root
        || created_repository.common_dir != repository.common_dir
        || created_repository.identity != repository.identity
    {
        return Err(
            "created SchoolX Code worktree failed repository identity validation".to_string(),
        );
    }
    let reserved_head = resolve_commit(&execution_root, "HEAD")?;
    if reserved_head != base_commit {
        return Err("created SchoolX Code worktree is not at the resolved base commit".to_string());
    }
    // `worktree add --no-checkout` creates only Git administrative metadata.
    // Enumerate effective config again from the new worktree context before
    // materializing tracked files so worktree-scoped and conditional filter
    // drivers are disabled by their exact keys.
    checkout_managed_worktree(&execution_root, base_commit, launch)?;

    Ok(CodeWorktreeDescriptor {
        execution_mode: CodeExecutionMode::Worktree,
        repository_identity: repository.identity.clone(),
        execution_root: path_to_string(&execution_root, "execution root")?,
        base_ref: base_commit.to_string(),
        worktree_id: Some(worktree_id),
    })
}

#[cfg(unix)]
fn add_managed_worktree(
    repository: &RepositoryInfo,
    target: &Path,
    base_commit: &str,
    launch: &PinnedGitLaunchAuthority,
) -> Result<(), String> {
    let request = PinnedGitRequest::WorktreeAdd {
        git_executable: path_to_string(&launch.git_executable()?, "git executable")?,
        git_common_dir: path_to_string(&repository.common_dir, "Git common directory")?,
        base_commit: base_commit.to_string(),
        disabled_filter_keys: repository_filter_overrides(&repository.top_level)?,
        expected_target_path: path_to_string(target, "managed worktree target")?,
    };
    run_git_in_pinned_directory(target, request, launch).map(|_| ())
}

#[cfg(unix)]
fn checkout_managed_worktree(
    execution_root: &Path,
    base_commit: &str,
    launch: &PinnedGitLaunchAuthority,
) -> Result<(), String> {
    let request = PinnedGitRequest::Checkout {
        git_executable: path_to_string(&launch.git_executable()?, "git executable")?,
        base_commit: base_commit.to_string(),
        disabled_filter_keys: repository_filter_overrides(execution_root)?,
        expected_target_path: path_to_string(execution_root, "managed worktree target")?,
    };
    run_git_in_pinned_directory(execution_root, request, launch).map(|_| ())
}

fn repository_descriptor(repository: &RepositoryInfo) -> Result<CodeRepositoryDescriptor, String> {
    Ok(CodeRepositoryDescriptor {
        repository_root: path_to_string(&repository.top_level, "repository root")?,
        git_common_dir: path_to_string(&repository.common_dir, "Git common directory")?,
        repository_identity: repository.identity.clone(),
    })
}

fn discover_repository(candidate: &Path) -> Result<RepositoryInfo, String> {
    discover_repository_until(candidate, None)
}

fn discover_repository_until(
    candidate: &Path,
    deadline: Option<Instant>,
) -> Result<RepositoryInfo, String> {
    let candidate = canonical_existing_directory(candidate, "Git repository path")?;
    #[cfg(unix)]
    let top_level_output = run_pinned_read_until(
        &candidate,
        CodePinnedReadCommand::TopLevel,
        Vec::new(),
        deadline,
    )?;
    #[cfg(not(unix))]
    let top_level_output = run_git_until(
        &candidate,
        &[
            OsString::from("rev-parse"),
            OsString::from("--path-format=absolute"),
            OsString::from("--show-toplevel"),
        ],
        GitOperation::ReadOnly,
        deadline,
    )?;
    let top_level = single_path_output(&top_level_output, "Git top-level")?;
    let top_level = canonical_existing_directory(&top_level, "Git top-level")?;

    #[cfg(unix)]
    let common_dir_output = run_pinned_read_until(
        &top_level,
        CodePinnedReadCommand::CommonDir,
        Vec::new(),
        deadline,
    )?;
    #[cfg(not(unix))]
    let common_dir_output = run_git_until(
        &top_level,
        &[
            OsString::from("rev-parse"),
            OsString::from("--path-format=absolute"),
            OsString::from("--git-common-dir"),
        ],
        GitOperation::ReadOnly,
        deadline,
    )?;
    let common_dir = single_path_output(&common_dir_output, "Git common directory")?;
    let common_dir = canonical_existing_directory(&common_dir, "Git common directory")?;
    let identity = repository_identity(&common_dir)?;

    Ok(RepositoryInfo {
        top_level,
        common_dir,
        identity,
    })
}

fn resolve_commit(repository_root: &Path, base_ref: &str) -> Result<String, String> {
    resolve_commit_until(repository_root, base_ref, None)
}

fn resolve_commit_until(
    repository_root: &Path,
    base_ref: &str,
    deadline: Option<Instant>,
) -> Result<String, String> {
    let base_ref = validate_base_ref(base_ref)?;
    #[cfg(unix)]
    let output = run_pinned_read_until(
        repository_root,
        CodePinnedReadCommand::ResolveCommit {
            base_ref: base_ref.to_string(),
        },
        Vec::new(),
        deadline,
    )
    .map_err(|error| format!("failed to resolve SchoolX Code base ref `{base_ref}`: {error}"))?;
    #[cfg(not(unix))]
    let output = {
        let revision = format!("{base_ref}^{{commit}}");
        run_git_until(
            repository_root,
            &[
                OsString::from("rev-parse"),
                OsString::from("--verify"),
                OsString::from("--quiet"),
                OsString::from("--end-of-options"),
                OsString::from(revision),
            ],
            GitOperation::ReadOnly,
            deadline,
        )
        .map_err(|error| format!("failed to resolve SchoolX Code base ref `{base_ref}`: {error}"))?
    };
    let commit = single_text_output(&output, "resolved Git commit")?;
    validate_commit_id(&commit)?;
    Ok(commit)
}

fn repository_is_dirty_until(
    repository_root: &Path,
    deadline: Option<Instant>,
) -> Result<bool, String> {
    #[cfg(unix)]
    let output = run_pinned_read_until(
        repository_root,
        CodePinnedReadCommand::StatusPorcelain,
        repository_filter_overrides_until(repository_root, deadline)?,
        deadline,
    )?;
    #[cfg(not(unix))]
    let output = run_git_until(
        repository_root,
        &[
            OsString::from("status"),
            OsString::from("--porcelain=v1"),
            OsString::from("-z"),
            OsString::from("--untracked-files=normal"),
        ],
        GitOperation::WorkingTreeRead,
        deadline,
    )?;
    Ok(!output.is_empty())
}

fn repository_branch_until(
    repository_root: &Path,
    deadline: Option<Instant>,
) -> Result<Option<String>, String> {
    #[cfg(unix)]
    let output = run_pinned_read_until(
        repository_root,
        CodePinnedReadCommand::CurrentBranch,
        Vec::new(),
        deadline,
    )?;
    #[cfg(not(unix))]
    let output = run_git_until(
        repository_root,
        &[OsString::from("branch"), OsString::from("--show-current")],
        GitOperation::ReadOnly,
        deadline,
    )?;
    let branch = std::str::from_utf8(&output)
        .map_err(|error| format!("current Git branch was not UTF-8: {error}"))?
        .trim_end_matches(['\r', '\n']);
    if branch.is_empty() {
        return Ok(None);
    }
    if branch.len() > MAX_BASE_REF_BYTES
        || branch
            .chars()
            .any(|character| character.is_control() || character == '\0')
    {
        return Err("current Git branch contained an unsafe value".to_string());
    }
    Ok(Some(branch.to_string()))
}

pub(crate) fn repository_identity(common_dir: &Path) -> Result<String, String> {
    let common_dir = path_to_string(common_dir, "Git common directory")?;
    let mut hasher = Sha256::new();
    hasher.update(b"schoolx-code-repository-v1\0");
    hasher.update(common_dir.as_bytes());
    Ok(hex::encode(hasher.finalize()))
}

fn validate_base_ref(value: &str) -> Result<&str, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_BASE_REF_BYTES {
        return Err(format!(
            "SchoolX Code base ref must be between 1 and {MAX_BASE_REF_BYTES} bytes"
        ));
    }
    if value.starts_with('-')
        || value
            .chars()
            .any(|character| character.is_control() || character == '\0')
    {
        return Err("SchoolX Code base ref contains unsafe characters".to_string());
    }
    Ok(value)
}

fn validate_repository_identity(value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("SchoolX Code repository identity must be 64 lowercase hex characters".into());
    }
    Ok(())
}

fn validate_commit_id(value: &str) -> Result<(), String> {
    if !matches!(value.len(), 40 | 64)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("SchoolX Code base commit must be a full lowercase Git object id".to_string());
    }
    Ok(())
}

fn validate_worktree_id(value: &str) -> Result<(), String> {
    let parsed = Uuid::parse_str(value)
        .map_err(|error| format!("invalid SchoolX Code worktree id: {error}"))?;
    if parsed.to_string() != value {
        return Err("SchoolX Code worktree id is not canonical".to_string());
    }
    Ok(())
}

fn canonical_existing_directory(path: &Path, label: &str) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err(format!("{label} must be an absolute path"));
    }
    let metadata = path
        .symlink_metadata()
        .map_err(|error| format!("failed to inspect {label} {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("{label} {} must not be a symlink", path.display()));
    }
    if !metadata.is_dir() {
        return Err(format!("{label} {} must be a directory", path.display()));
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize {label} {}: {error}", path.display()))?;
    let canonical_metadata = canonical.symlink_metadata().map_err(|error| {
        format!(
            "failed to revalidate canonical {label} {}: {error}",
            canonical.display()
        )
    })?;
    if canonical_metadata.file_type().is_symlink() || !canonical_metadata.is_dir() {
        return Err(format!(
            "canonical {label} {} is not a real directory",
            canonical.display()
        ));
    }
    Ok(canonical)
}

fn ensure_real_child_directory(parent: &Path, name: &str) -> Result<PathBuf, String> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        return Err("invalid SchoolX Code managed-directory component".to_string());
    }
    let child = parent.join(name);
    match child.symlink_metadata() {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "SchoolX Code managed directory {} must not be a symlink",
                    child.display()
                ));
            }
            if !metadata.is_dir() {
                return Err(format!(
                    "SchoolX Code managed path {} must be a directory",
                    child.display()
                ));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match create_private_directory(&child) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(format!(
                        "failed to create SchoolX Code managed directory {}: {error}",
                        child.display()
                    ));
                }
            }
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect SchoolX Code managed directory {}: {error}",
                child.display()
            ));
        }
    }
    let canonical = canonical_existing_directory(&child, "SchoolX Code managed directory")?;
    if canonical.parent() != Some(parent) {
        return Err(format!(
            "SchoolX Code managed directory {} escaped its parent",
            child.display()
        ));
    }
    // Existing buckets may have been created by an older build or a permissive
    // umask. Re-apply the product boundary on every use, not just creation.
    set_owner_only_directory(&canonical)?;
    Ok(canonical)
}

fn reserve_worktree_target(parent: &Path) -> Result<(String, PathBuf), String> {
    for _ in 0..WORKTREE_ID_ATTEMPTS {
        let worktree_id = Uuid::new_v4().to_string();
        let target = parent.join(&worktree_id);
        match create_private_directory(&target) {
            Ok(()) => {
                validate_reserved_worktree_target(parent, &target)?;
                return Ok((worktree_id, target));
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(format!(
                    "failed to reserve SchoolX Code worktree target {}: {error}",
                    target.display()
                ));
            }
        }
    }
    Err("failed to allocate an unused SchoolX Code worktree id".to_string())
}

fn validate_reserved_worktree_target(parent: &Path, target: &Path) -> Result<(), String> {
    let metadata = target.symlink_metadata().map_err(|error| {
        format!(
            "failed to inspect reserved SchoolX Code worktree target {}: {error}",
            target.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "reserved SchoolX Code worktree target {} is not a real directory",
            target.display()
        ));
    }
    let canonical = target.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize reserved SchoolX Code worktree target {}: {error}",
            target.display()
        )
    })?;
    if canonical != target || canonical.parent() != Some(parent) {
        return Err(format!(
            "reserved SchoolX Code worktree target {} escaped its parent",
            target.display()
        ));
    }
    let mut entries = fs::read_dir(&canonical).map_err(|error| {
        format!(
            "failed to read reserved SchoolX Code worktree target {}: {error}",
            canonical.display()
        )
    })?;
    match entries.next() {
        None => Ok(()),
        Some(Ok(_)) => Err(format!(
            "reserved SchoolX Code worktree target {} is not empty",
            canonical.display()
        )),
        Some(Err(error)) => Err(format!(
            "failed to inspect reserved SchoolX Code worktree target {}: {error}",
            canonical.display()
        )),
    }
}

#[cfg(unix)]
fn create_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private_directory(path: &Path) -> io::Result<()> {
    fs::create_dir(path)
}

fn validate_managed_execution_root(
    nest_root: &Path,
    repository_identity: &str,
    worktree_id: &str,
    claimed_root: &Path,
) -> Result<PathBuf, String> {
    validate_repository_identity(repository_identity)?;
    validate_worktree_id(worktree_id)?;
    let nest_root = canonical_existing_directory(nest_root, "SchoolX nest root")?;
    let worktrees_root = existing_real_child_directory(&nest_root, WORKTREES_DIRECTORY)?;
    let repository_root = existing_real_child_directory(&worktrees_root, repository_identity)?;
    let expected_root = repository_root.join(worktree_id);
    if claimed_root != expected_root {
        return Err("SchoolX Code worktree path does not match its managed descriptor".to_string());
    }
    let execution_root = existing_real_child_directory(&repository_root, worktree_id)?;
    if execution_root != expected_root || !execution_root.starts_with(&worktrees_root) {
        return Err("SchoolX Code worktree escaped the active nest boundary".to_string());
    }
    Ok(execution_root)
}

fn existing_real_child_directory(parent: &Path, name: &str) -> Result<PathBuf, String> {
    let child = parent.join(name);
    let canonical = canonical_existing_directory(&child, "SchoolX Code managed directory")?;
    if canonical.parent() != Some(parent) {
        return Err(format!(
            "SchoolX Code managed directory {} escaped its parent",
            child.display()
        ));
    }
    Ok(canonical)
}

#[cfg(unix)]
fn set_owner_only_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::fs::PermissionsExt;

    let directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .map_err(|error| format!("failed to securely open {}: {error}", path.display()))?;
    let metadata = directory.metadata().map_err(|error| {
        format!(
            "failed to inspect open directory {}: {error}",
            path.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!("{} is not a directory", path.display()));
    }
    directory
        .set_permissions(fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("failed to restrict {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn set_owner_only_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn path_to_string(path: &Path, label: &str) -> Result<String, String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{label} is not valid UTF-8"))
}

fn single_path_output(output: &[u8], label: &str) -> Result<PathBuf, String> {
    single_text_output(output, label).map(PathBuf::from)
}

fn single_text_output(output: &[u8], label: &str) -> Result<String, String> {
    let output =
        std::str::from_utf8(output).map_err(|error| format!("{label} was not UTF-8: {error}"))?;
    let output = output.trim_end_matches(['\r', '\n']);
    if output.is_empty() || output.contains('\n') || output.contains('\r') {
        return Err(format!("{label} did not contain exactly one value"));
    }
    Ok(output.to_string())
}

#[cfg(not(unix))]
fn run_git(cwd: &Path, args: &[OsString], operation: GitOperation) -> Result<Vec<u8>, String> {
    run_git_until(cwd, args, operation, None)
}

#[cfg(not(unix))]
fn run_git_until(
    cwd: &Path,
    args: &[OsString],
    operation: GitOperation,
    deadline: Option<Instant>,
) -> Result<Vec<u8>, String> {
    let disabled_filter_keys = if operation.may_run_repository_filters() {
        repository_filter_overrides_until(cwd, deadline)?
    } else {
        Vec::new()
    };
    run_git_with_filter_overrides_until(cwd, args, operation, &disabled_filter_keys, deadline)
}

fn repository_filter_overrides(cwd: &Path) -> Result<Vec<String>, String> {
    repository_filter_overrides_until(cwd, None)
}

fn repository_filter_overrides_until(
    cwd: &Path,
    deadline: Option<Instant>,
) -> Result<Vec<String>, String> {
    let mut overrides = BTreeSet::new();
    #[cfg(unix)]
    let local_output = run_pinned_read_until(
        cwd,
        CodePinnedReadCommand::LocalConfig,
        Vec::new(),
        deadline,
    )?;
    #[cfg(not(unix))]
    let local_output = run_git_with_filter_overrides_until(
        cwd,
        &[
            OsString::from("config"),
            OsString::from("--local"),
            OsString::from("--includes"),
            OsString::from("--null"),
            OsString::from("--list"),
        ],
        GitOperation::ReadOnly,
        &[],
        deadline,
    )?;
    let worktree_config_enabled = collect_local_filter_overrides(&local_output, &mut overrides)?;
    if worktree_config_enabled {
        #[cfg(unix)]
        let worktree_output = run_pinned_read_until(
            cwd,
            CodePinnedReadCommand::WorktreeConfigNames,
            Vec::new(),
            deadline,
        )?;
        #[cfg(not(unix))]
        let worktree_output = run_git_with_filter_overrides_until(
            cwd,
            &[
                OsString::from("config"),
                OsString::from("--worktree"),
                OsString::from("--includes"),
                OsString::from("--null"),
                OsString::from("--name-only"),
                OsString::from("--list"),
            ],
            GitOperation::ReadOnly,
            &[],
            deadline,
        )?;
        collect_filter_override_names(&worktree_output, &mut overrides)?;
    }
    Ok(overrides.into_iter().collect())
}

pub(crate) fn collect_local_filter_overrides(
    output: &[u8],
    overrides: &mut BTreeSet<String>,
) -> Result<bool, String> {
    let mut worktree_config_enabled = false;
    for record in output
        .split(|byte| *byte == b'\0')
        .filter(|record| !record.is_empty())
    {
        let separator = record
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or_else(|| "Git config record did not contain a value separator".to_string())?;
        let key = std::str::from_utf8(&record[..separator])
            .map_err(|error| format!("Git config key was not UTF-8: {error}"))?;
        collect_filter_override(key, overrides)?;
        if key.eq_ignore_ascii_case("extensions.worktreeConfig") {
            worktree_config_enabled = parse_git_boolean(&record[separator + 1..])?;
        }
    }
    Ok(worktree_config_enabled)
}

pub(crate) fn collect_filter_override_names(
    output: &[u8],
    overrides: &mut BTreeSet<String>,
) -> Result<(), String> {
    for raw_key in output
        .split(|byte| *byte == b'\0')
        .filter(|key| !key.is_empty())
    {
        let key = std::str::from_utf8(raw_key)
            .map_err(|error| format!("Git config key was not UTF-8: {error}"))?;
        collect_filter_override(key, overrides)?;
    }
    Ok(())
}

fn collect_filter_override(key: &str, overrides: &mut BTreeSet<String>) -> Result<(), String> {
    let normalized = key.to_ascii_lowercase();
    let is_filter_command = normalized.starts_with("filter.")
        && [".clean", ".smudge", ".process"]
            .iter()
            .any(|suffix| normalized.ends_with(suffix));
    if !is_filter_command {
        return Ok(());
    }
    if key.chars().any(char::is_control) {
        return Err("Git filter config key contained control characters".to_string());
    }
    let section_end = key
        .rfind('.')
        .ok_or_else(|| "Git filter config key was malformed".to_string())?;
    let filter_prefix = &key[..section_end];
    overrides.insert(key.to_string());
    overrides.insert(format!("{filter_prefix}.required"));
    Ok(())
}

fn parse_git_boolean(value: &[u8]) -> Result<bool, String> {
    let value = std::str::from_utf8(value)
        .map_err(|error| format!("Git worktreeConfig value was not UTF-8: {error}"))?;
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        _ => Err("Git extensions.worktreeConfig was not a valid boolean".to_string()),
    }
}

#[cfg(not(unix))]
fn run_git_with_filter_overrides_until(
    cwd: &Path,
    args: &[OsString],
    operation: GitOperation,
    disabled_filter_keys: &[String],
    deadline: Option<Instant>,
) -> Result<Vec<u8>, String> {
    run_git_executable_with_filter_overrides(
        &git_executable()?,
        cwd,
        args,
        operation,
        disabled_filter_keys,
        deadline,
    )
}

#[cfg(any(not(unix), test))]
fn git_executable() -> Result<PathBuf, String> {
    let executable = crate::managed_agents::resolve_command("git")
        .ok_or_else(|| "git executable was not found".to_string())?;
    executable
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize git executable: {error}"))
}

#[cfg(unix)]
fn run_git_in_pinned_directory(
    directory_path: &Path,
    request: PinnedGitRequest,
    launch: &PinnedGitLaunchAuthority,
) -> Result<Vec<u8>, String> {
    let operation = prepare_pinned_git_operation_with_launch(directory_path, request, launch)?;
    run_git_with_pinned_operation(operation)
}

#[cfg(unix)]
fn prepare_pinned_git_operation(
    directory_path: &Path,
    mut request: PinnedGitRequest,
) -> Result<CodePinnedGitOperation, String> {
    let directories = pin_pinned_git_directories(directory_path, &request)?;
    let target = directories
        .last()
        .ok_or_else(|| "pinned Git operation did not carry its target handle".to_string())?;
    let launch = PinnedGitLaunchAuthority::admit(target)?;
    launch.bind_request(&mut request)?;
    verify_pinned_target_chain(&request, &directories)?;
    Ok(CodePinnedGitOperation {
        request,
        directories,
        launch,
    })
}

#[cfg(unix)]
fn prepare_pinned_git_operation_with_launch(
    directory_path: &Path,
    mut request: PinnedGitRequest,
    launch: &PinnedGitLaunchAuthority,
) -> Result<CodePinnedGitOperation, String> {
    let directories = pin_pinned_git_directories(directory_path, &request)?;
    launch.bind_request(&mut request)?;
    verify_pinned_target_chain(&request, &directories)?;
    Ok(CodePinnedGitOperation {
        request,
        directories,
        launch: launch.clone(),
    })
}

#[cfg(unix)]
fn pin_pinned_git_directories(
    directory_path: &Path,
    request: &PinnedGitRequest,
) -> Result<Vec<fs::File>, String> {
    let expected_target = request.expected_target_path();
    if expected_target != directory_path {
        return Err("pinned Git target did not match its native request".to_string());
    }
    let target = pin_git_directory(directory_path)?;
    if matches!(request, PinnedGitRequest::ReadOnly { .. }) {
        let directories = vec![target];
        verify_pinned_target_chain(request, &directories)?;
        return Ok(directories);
    }
    let repository_bucket = directory_path
        .parent()
        .ok_or_else(|| "managed Git target had no repository bucket".to_string())?;
    let worktrees_root = repository_bucket
        .parent()
        .ok_or_else(|| "managed Git target had no WORKTREES root".to_string())?;
    let nest_root = worktrees_root
        .parent()
        .ok_or_else(|| "managed Git target had no nest root".to_string())?;
    let directories = vec![
        pin_git_directory(nest_root)?,
        pin_git_directory(worktrees_root)?,
        pin_git_directory(repository_bucket)?,
        target,
    ];
    verify_pinned_target_chain(request, &directories)?;
    Ok(directories)
}

#[cfg(unix)]
fn pin_git_directory(directory_path: &Path) -> Result<fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt;

    let directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(directory_path)
        .map_err(|error| {
            format!(
                "failed to pin SchoolX Code Git directory {}: {error}",
                directory_path.display()
            )
        })?;
    if !directory
        .metadata()
        .map_err(|error| format!("failed to inspect pinned Git directory: {error}"))?
        .is_dir()
    {
        return Err("pinned SchoolX Code Git target is not a directory".to_string());
    }
    Ok(directory)
}

#[cfg(unix)]
fn run_git_with_pinned_operation(operation: CodePinnedGitOperation) -> Result<Vec<u8>, String> {
    operation.execute()
}

/// Test-only subprocess entry that preserves crash/race regression coverage
/// without participating in production launch authority.
#[cfg(all(unix, test))]
fn execute_pinned_git_helper() -> Result<(), String> {
    use std::os::fd::AsFd;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::process::CommandExt;

    let encoded = std::env::var(PINNED_GIT_REQUEST_ENV)
        .map_err(|_| "pinned Git helper request was missing or not UTF-8".to_string())?;
    if encoded.len() > MAX_PINNED_GIT_REQUEST_BYTES {
        return Err(format!(
            "pinned Git helper request exceeded {MAX_PINNED_GIT_REQUEST_BYTES} bytes"
        ));
    }
    let envelope: PinnedGitEnvelope = serde_json::from_str(&encoded)
        .map_err(|error| format!("invalid pinned Git helper request: {error}"))?;
    validate_pinned_git_envelope(&envelope)?;

    let stdin = std::io::stdin();
    let stat = rustix::fs::fstat(stdin.as_fd())
        .map_err(|error| format!("failed to inspect pinned Git helper directory: {error}"))?;
    if !rustix::fs::FileType::from_raw_mode(stat.st_mode).is_dir()
        || stat.st_dev as u64 != envelope.target_device
        || stat.st_ino as u64 != envelope.target_inode
    {
        return Err("pinned Git helper directory identity did not match its request".to_string());
    }
    rustix::process::fchdir(stdin.as_fd())
        .map_err(|error| format!("failed to enter pinned Git directory: {error}"))?;
    let current = fs::metadata(".")
        .map_err(|error| format!("failed to verify pinned Git working directory: {error}"))?;
    if current.dev() != envelope.target_device || current.ino() != envelope.target_inode {
        return Err("pinned Git helper changed to a different directory".to_string());
    }

    let pinned_untracked_file = match &envelope.request {
        PinnedGitRequest::ReadOnly {
            command: CodePinnedReadCommand::UntrackedPatch { path },
            ..
        } => Some(open_pinned_untracked_file(stdin.as_fd(), path)?),
        _ => None,
    };
    let mut command = pinned_git_command(&envelope.request)?;
    if let Some(file) = pinned_untracked_file {
        command.stdin(Stdio::from(file));
    } else {
        command.stdin(Stdio::null());
    }
    let error = command.exec();
    Err(format!("failed to execute pinned Git: {error}"))
}

#[cfg(unix)]
fn open_pinned_untracked_file(
    root: std::os::fd::BorrowedFd<'_>,
    relative: &str,
) -> Result<fs::File, String> {
    use std::os::fd::AsFd;
    use std::path::Component;

    let components = Path::new(relative)
        .components()
        .map(|component| match component {
            Component::Normal(value) => Ok(value),
            _ => Err("pinned untracked path was not repository-relative".to_string()),
        })
        .collect::<Result<Vec<_>, String>>()?;
    let (file_name, ancestors) = components
        .split_last()
        .ok_or_else(|| "pinned untracked path was empty".to_string())?;
    let mut directories = Vec::with_capacity(ancestors.len());
    for component in ancestors {
        let parent = directories
            .last()
            .map_or(root, |directory: &rustix::fd::OwnedFd| directory.as_fd());
        let directory = rustix::fs::openat(
            parent,
            *component,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|error| format!("failed to pin untracked path ancestor: {error}"))?;
        directories.push(directory);
    }
    let parent = directories
        .last()
        .map_or(root, |directory| directory.as_fd());
    let file = rustix::fs::openat(
        parent,
        *file_name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| format!("failed to pin untracked file: {error}"))?;
    let stat = rustix::fs::fstat(&file)
        .map_err(|error| format!("failed to inspect pinned untracked file: {error}"))?;
    if !rustix::fs::FileType::from_raw_mode(stat.st_mode).is_file() {
        return Err("pinned untracked entry is not a regular file".to_string());
    }
    Ok(fs::File::from(file))
}

#[cfg(unix)]
fn validate_pinned_git_envelope(envelope: &PinnedGitEnvelope) -> Result<(), String> {
    if envelope.version != PINNED_GIT_REQUEST_VERSION {
        return Err(format!(
            "unsupported pinned Git request version {}",
            envelope.version
        ));
    }
    let expected_target = envelope.request.expected_target_path();
    if !expected_target.is_absolute() {
        return Err("pinned Git target path must be absolute".to_string());
    }
    if expected_target.to_string_lossy().len() > MAX_PINNED_GIT_PATH_BYTES {
        return Err("pinned Git target path exceeded its safety limit".to_string());
    }
    match &envelope.request {
        PinnedGitRequest::WorktreeAdd {
            git_executable,
            git_common_dir,
            base_commit,
            disabled_filter_keys,
            ..
        } => {
            validate_helper_path_length(git_executable)?;
            validate_helper_path_length(git_common_dir)?;
            canonical_existing_directory(Path::new(git_common_dir), "pinned Git common directory")?;
            validate_commit_id(base_commit)?;
            validate_filter_override_keys(disabled_filter_keys)
        }
        PinnedGitRequest::Checkout {
            git_executable,
            base_commit,
            disabled_filter_keys,
            ..
        } => {
            validate_helper_path_length(git_executable)?;
            validate_commit_id(base_commit)?;
            validate_filter_override_keys(disabled_filter_keys)
        }
        PinnedGitRequest::ReadOnly {
            git_executable,
            command,
            disabled_filter_keys,
            ..
        } => {
            validate_helper_path_length(git_executable)?;
            validate_pinned_read_command(command)?;
            validate_filter_override_keys(disabled_filter_keys)
        }
    }
}

/// Decode and revalidate one closed pinned-Git envelope inside the signed
/// macOS service, then derive its fixed `/usr/bin/git` process specification.
#[cfg(target_os = "macos")]
pub(in crate::code_workspace) fn prepare_macos_pinned_git(
    payload: &str,
    cwd: DescriptorObservation,
    stdin: DescriptorObservation,
) -> Result<MacGitProcessSpec, String> {
    if payload.len() > MAX_PINNED_GIT_REQUEST_BYTES {
        return Err(format!(
            "pinned Git request exceeded {MAX_PINNED_GIT_REQUEST_BYTES} bytes"
        ));
    }
    let envelope: PinnedGitEnvelope = serde_json::from_str(payload)
        .map_err(|error| format!("invalid pinned Git helper request: {error}"))?;
    validate_pinned_git_envelope(&envelope)?;
    macos_git_xpc::validate_directory_observation(
        cwd,
        envelope.target_device,
        envelope.target_inode,
        None,
        "pinned Git cwd",
    )?;
    if envelope.request.git_executable() != Path::new(MACOS_SYSTEM_GIT) {
        return Err("macOS pinned Git request did not select /usr/bin/git".to_string());
    }
    let trusted_git = super::git_write::macos_root_trusted_git()?;
    match &envelope.request {
        PinnedGitRequest::ReadOnly {
            command: CodePinnedReadCommand::UntrackedPatch { .. },
            ..
        } => macos_git_xpc::validate_bounded_regular_observation(
            stdin,
            u64::MAX,
            "pinned untracked input",
        )?,
        _ => macos_git_xpc::validate_null_observation(stdin, "pinned Git input")?,
    }
    let mut command = Command::new(trusted_git);
    configure_pinned_git_command(&mut command, &envelope.request)?;
    macos_git_xpc::process_spec_from_command(&command)
}

#[cfg(unix)]
fn validate_pinned_read_command(command: &CodePinnedReadCommand) -> Result<(), String> {
    match command {
        CodePinnedReadCommand::TopLevel
        | CodePinnedReadCommand::CommonDir
        | CodePinnedReadCommand::LocalConfig
        | CodePinnedReadCommand::WorktreeConfigNames
        | CodePinnedReadCommand::HeadCommit
        | CodePinnedReadCommand::CurrentBranch
        | CodePinnedReadCommand::StatusPorcelain
        | CodePinnedReadCommand::TrackedUnmergedPaths => Ok(()),
        CodePinnedReadCommand::ResolveCommit { base_ref } => {
            validate_base_ref(base_ref).map(|_| ())
        }
        CodePinnedReadCommand::DirectLocalRefCommit { target_ref } => {
            validate_direct_local_branch_ref(target_ref)
        }
        CodePinnedReadCommand::MergeBaseIsAncestor {
            head_commit,
            target_commit,
        } => {
            validate_commit_id(head_commit)?;
            validate_commit_id(target_commit)
        }
        CodePinnedReadCommand::VerifyCommit { commit }
        | CodePinnedReadCommand::TrackedNumstat {
            base_commit: commit,
        }
        | CodePinnedReadCommand::TrackedNameStatus {
            base_commit: commit,
        } => validate_commit_id(commit),
        CodePinnedReadCommand::TrackedPatch { base_commit, path } => {
            validate_commit_id(base_commit)?;
            validate_pinned_read_path(path)
        }
        CodePinnedReadCommand::UntrackedPaths => Ok(()),
        CodePinnedReadCommand::UntrackedPatch { path } => validate_pinned_read_path(path),
    }
}

#[cfg(unix)]
fn validate_pinned_read_path(value: &str) -> Result<(), String> {
    use std::path::Component;

    if value.is_empty()
        || value.len() > MAX_PINNED_GIT_PATH_BYTES
        || value.chars().any(char::is_control)
        || !Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err("pinned read-only Git path was invalid".to_string());
    }
    Ok(())
}

#[cfg(unix)]
fn validate_helper_path_length(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > MAX_PINNED_GIT_PATH_BYTES {
        Err("pinned Git path exceeded its safety limit".to_string())
    } else {
        Ok(())
    }
}

#[cfg(all(unix, test))]
fn pinned_git_command(request: &PinnedGitRequest) -> Result<Command, String> {
    let git_executable = validate_helper_git_executable(
        request
            .git_executable()
            .to_str()
            .ok_or_else(|| "pinned Git executable was not UTF-8".to_string())?,
    )?;
    let mut command = Command::new(git_executable);
    configure_pinned_git_command(&mut command, request)?;
    Ok(command)
}

#[cfg(unix)]
fn configure_pinned_git_command(
    command: &mut Command,
    request: &PinnedGitRequest,
) -> Result<(), String> {
    let (arguments, disabled_filter_keys): (Vec<OsString>, &[String]) = match request {
        PinnedGitRequest::WorktreeAdd {
            git_common_dir,
            base_commit,
            disabled_filter_keys,
            ..
        } => {
            let git_common_dir = canonical_existing_directory(
                Path::new(git_common_dir),
                "pinned Git common directory",
            )?;
            (
                vec![
                    OsString::from(format!(
                        "--git-dir={}",
                        path_to_string(&git_common_dir, "pinned Git common directory")?
                    )),
                    OsString::from("worktree"),
                    OsString::from("add"),
                    OsString::from("--detach"),
                    OsString::from("--no-checkout"),
                    OsString::from("--"),
                    OsString::from("."),
                    OsString::from(base_commit),
                ],
                disabled_filter_keys,
            )
        }
        PinnedGitRequest::Checkout {
            base_commit,
            disabled_filter_keys,
            ..
        } => (
            vec![
                OsString::from("checkout"),
                OsString::from("--detach"),
                OsString::from(base_commit),
            ],
            disabled_filter_keys,
        ),
        PinnedGitRequest::ReadOnly {
            command,
            disabled_filter_keys,
            ..
        } => (pinned_read_arguments(command), disabled_filter_keys),
    };
    command.arg("--no-pager").args(arguments);
    let operation = match request {
        PinnedGitRequest::ReadOnly {
            command:
                CodePinnedReadCommand::StatusPorcelain
                | CodePinnedReadCommand::TrackedNumstat { .. }
                | CodePinnedReadCommand::TrackedNameStatus { .. }
                | CodePinnedReadCommand::TrackedUnmergedPaths
                | CodePinnedReadCommand::TrackedPatch { .. }
                | CodePinnedReadCommand::UntrackedPaths
                | CodePinnedReadCommand::UntrackedPatch { .. },
            ..
        } => GitOperation::WorkingTreeRead,
        PinnedGitRequest::ReadOnly { .. } => GitOperation::ReadOnly,
        PinnedGitRequest::WorktreeAdd { .. } | PinnedGitRequest::Checkout { .. } => {
            GitOperation::Mutating
        }
    };
    configure_git_environment(command, operation, disabled_filter_keys);
    if matches!(
        request,
        PinnedGitRequest::ReadOnly {
            command: CodePinnedReadCommand::UntrackedPatch { .. },
            ..
        }
    ) {
        command.env("LANG", "C").env("LC_ALL", "C");
    }
    command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
    crate::util::configure_no_window(command);
    Ok(())
}

#[cfg(unix)]
fn pinned_read_arguments(command: &CodePinnedReadCommand) -> Vec<OsString> {
    let literal = OsString::from("--literal-pathspecs");
    match command {
        CodePinnedReadCommand::TopLevel => vec![
            OsString::from("rev-parse"),
            OsString::from("--path-format=absolute"),
            OsString::from("--show-toplevel"),
        ],
        CodePinnedReadCommand::CommonDir => vec![
            OsString::from("rev-parse"),
            OsString::from("--path-format=absolute"),
            OsString::from("--git-common-dir"),
        ],
        CodePinnedReadCommand::LocalConfig => vec![
            OsString::from("config"),
            OsString::from("--local"),
            OsString::from("--includes"),
            OsString::from("--null"),
            OsString::from("--list"),
        ],
        CodePinnedReadCommand::WorktreeConfigNames => vec![
            OsString::from("config"),
            OsString::from("--worktree"),
            OsString::from("--includes"),
            OsString::from("--null"),
            OsString::from("--name-only"),
            OsString::from("--list"),
        ],
        CodePinnedReadCommand::ResolveCommit { base_ref } => vec![
            OsString::from("rev-parse"),
            OsString::from("--verify"),
            OsString::from("--quiet"),
            OsString::from("--end-of-options"),
            OsString::from(format!("{base_ref}^{{commit}}")),
        ],
        CodePinnedReadCommand::VerifyCommit { commit } => vec![
            OsString::from("rev-parse"),
            OsString::from("--verify"),
            OsString::from("--quiet"),
            OsString::from(commit),
        ],
        CodePinnedReadCommand::HeadCommit => vec![
            OsString::from("rev-parse"),
            OsString::from("--verify"),
            OsString::from("--quiet"),
            OsString::from("--end-of-options"),
            OsString::from("HEAD^{commit}"),
        ],
        CodePinnedReadCommand::CurrentBranch => {
            vec![OsString::from("branch"), OsString::from("--show-current")]
        }
        CodePinnedReadCommand::StatusPorcelain => vec![
            OsString::from("status"),
            OsString::from("--porcelain=v1"),
            OsString::from("-z"),
            OsString::from("--untracked-files=normal"),
        ],
        CodePinnedReadCommand::DirectLocalRefCommit { target_ref } => vec![
            OsString::from("rev-parse"),
            OsString::from("--verify"),
            OsString::from("--quiet"),
            OsString::from("--end-of-options"),
            OsString::from(format!("{target_ref}^{{commit}}")),
        ],
        CodePinnedReadCommand::MergeBaseIsAncestor {
            head_commit,
            target_commit,
        } => vec![
            OsString::from("merge-base"),
            OsString::from("--is-ancestor"),
            OsString::from("--end-of-options"),
            OsString::from(head_commit),
            OsString::from(target_commit),
        ],
        CodePinnedReadCommand::TrackedNumstat { base_commit } => vec![
            literal,
            OsString::from("diff"),
            OsString::from("--no-ext-diff"),
            OsString::from("--no-textconv"),
            OsString::from("--no-renames"),
            OsString::from("--numstat"),
            OsString::from("-z"),
            OsString::from(base_commit),
            OsString::from("--"),
        ],
        CodePinnedReadCommand::TrackedNameStatus { base_commit } => vec![
            literal,
            OsString::from("diff"),
            OsString::from("--no-ext-diff"),
            OsString::from("--no-textconv"),
            OsString::from("--no-renames"),
            OsString::from("--name-status"),
            OsString::from("-z"),
            OsString::from(base_commit),
            OsString::from("--"),
        ],
        CodePinnedReadCommand::TrackedUnmergedPaths => vec![
            literal,
            OsString::from("diff"),
            OsString::from("--no-ext-diff"),
            OsString::from("--no-textconv"),
            OsString::from("--no-renames"),
            OsString::from("--name-only"),
            OsString::from("--diff-filter=U"),
            OsString::from("-z"),
            OsString::from("--"),
        ],
        CodePinnedReadCommand::TrackedPatch { base_commit, path } => vec![
            literal,
            OsString::from("diff"),
            OsString::from("--no-ext-diff"),
            OsString::from("--no-textconv"),
            OsString::from("--no-renames"),
            OsString::from("--unified=80"),
            OsString::from("--src-prefix=a/"),
            OsString::from("--dst-prefix=b/"),
            OsString::from(base_commit),
            OsString::from("--"),
            OsString::from(path),
        ],
        CodePinnedReadCommand::UntrackedPaths => vec![
            literal,
            OsString::from("ls-files"),
            OsString::from("--others"),
            OsString::from("--exclude-standard"),
            OsString::from("-z"),
            OsString::from("--"),
        ],
        CodePinnedReadCommand::UntrackedPatch { path: _ } => vec![
            literal,
            OsString::from("diff"),
            OsString::from("--no-index"),
            OsString::from("--no-ext-diff"),
            OsString::from("--no-textconv"),
            OsString::from("--patch"),
            OsString::from("--unified=80"),
            OsString::from("--src-prefix=a/"),
            OsString::from("--dst-prefix=b/"),
            OsString::from("--"),
            OsString::from("/dev/null"),
            OsString::from("/dev/fd/0"),
        ],
    }
}

#[cfg(unix)]
fn spawn_pinned_git_helper(
    request: &PinnedGitRequest,
    directories: &[fs::File],
    launch: &PinnedGitLaunchAuthority,
) -> Result<Vec<u8>, String> {
    let target = directories
        .last()
        .ok_or_else(|| "pinned Git operation did not carry its target handle".to_string())?;
    let child = launch.spawn(request, target)?;
    #[cfg(all(target_os = "macos", not(test)))]
    return capture_macos_pinned_child(child, "pinned git", GIT_TIMEOUT);
    #[cfg(any(not(target_os = "macos"), test))]
    capture_child(child, "pinned git", GIT_TIMEOUT)
}

#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
fn prepare_pinned_read_operation(
    execution_root: &Path,
    command: CodePinnedReadCommand,
    disabled_filter_keys: Vec<String>,
) -> Result<CodePinnedGitOperation, String> {
    let request = PinnedGitRequest::ReadOnly {
        // Admission binds this placeholder to the platform's root-trusted Git
        // executable before the envelope is validated or launched.
        git_executable: String::new(),
        command,
        disabled_filter_keys,
        expected_target_path: path_to_string(execution_root, "pinned read-only Git root")?,
    };
    prepare_pinned_git_operation(execution_root, request)
}

#[cfg(unix)]
fn run_pinned_read_until(
    execution_root: &Path,
    command: CodePinnedReadCommand,
    disabled_filter_keys: Vec<String>,
    deadline: Option<Instant>,
) -> Result<Vec<u8>, String> {
    remaining_git_timeout(deadline)?;
    let operation = prepare_pinned_read_operation(execution_root, command, disabled_filter_keys)?;
    verify_pinned_target_chain(&operation.request, &operation.directories)?;
    let timeout = remaining_git_timeout(deadline)?;
    let target = operation
        .directories
        .last()
        .ok_or_else(|| "pinned Git operation did not carry its target handle".to_string())?;
    let child = operation.launch.spawn(&operation.request, target)?;
    #[cfg(all(target_os = "macos", not(test)))]
    let result = capture_macos_pinned_child(child, "pinned read-only git", timeout);
    #[cfg(any(not(target_os = "macos"), test))]
    let result = capture_child(child, "pinned read-only git", timeout);
    let verified = verify_pinned_target_chain(&operation.request, &operation.directories);
    match (result, verified) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(output), Ok(())) => Ok(strip_pinned_test_harness_output(output)),
    }
}

#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
fn run_pinned_read_before(
    execution_root: &Path,
    command: CodePinnedReadCommand,
    deadline: Instant,
) -> Result<Vec<u8>, String> {
    run_pinned_read_until(execution_root, command, Vec::new(), Some(deadline))
}

#[cfg(unix)]
#[cfg_attr(not(test), allow(dead_code))]
fn run_pinned_ancestry_before(
    execution_root: &Path,
    head_commit: &str,
    target_commit: &str,
    deadline: Instant,
) -> Result<bool, String> {
    remaining_git_timeout(Some(deadline))?;
    let operation = prepare_pinned_read_operation(
        execution_root,
        CodePinnedReadCommand::MergeBaseIsAncestor {
            head_commit: head_commit.to_string(),
            target_commit: target_commit.to_string(),
        },
        Vec::new(),
    )?;
    verify_pinned_target_chain(&operation.request, &operation.directories)?;
    let timeout = remaining_git_timeout(Some(deadline))?;
    let target = operation
        .directories
        .last()
        .ok_or_else(|| "pinned Git operation did not carry its target handle".to_string())?;
    let mut child = operation.launch.spawn(&operation.request, target)?;
    #[cfg(all(target_os = "macos", not(test)))]
    let captured = capture_macos_pinned_child_status(&mut child, "pinned merge-base git", timeout);
    #[cfg(any(not(target_os = "macos"), test))]
    let captured = capture_child_status(&mut child, "pinned merge-base git", timeout);
    let verified = verify_pinned_target_chain(&operation.request, &operation.directories);
    let mut captured = match (captured, verified) {
        (Err(error), _) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
        (Ok(captured), Ok(())) => captured,
    };
    captured.stderr.bytes = strip_linux_onnxruntime_startup_diagnostic(captured.stderr.bytes);
    if captured.stdout.truncated || captured.stderr.truncated {
        return Err("pinned merge-base output exceeded the SchoolX Code limit".to_string());
    }
    let stdout = strip_pinned_test_harness_output(captured.stdout.bytes.clone());
    match captured.status.code() {
        Some(0) if stdout.is_empty() && captured.stderr.bytes.is_empty() => Ok(true),
        Some(1) if stdout.is_empty() && captured.stderr.bytes.is_empty() => Ok(false),
        _ => Err(captured_child_error("pinned merge-base git", &captured)),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn strip_pinned_test_harness_output(output: Vec<u8>) -> Vec<u8> {
    #[cfg(test)]
    {
        output
            .strip_prefix(b"\nrunning 1 test\n")
            .unwrap_or(&output)
            .to_vec()
    }
    #[cfg(not(test))]
    output
}

#[cfg(unix)]
pub(crate) fn strip_linux_onnxruntime_startup_diagnostic(output: Vec<u8>) -> Vec<u8> {
    #[cfg(target_os = "linux")]
    {
        // The statically linked ONNX runtime can emit this before `main` on
        // virtualized ARM Linux. Remove only one exact, complete first line;
        // helper/Git stderr after it remains fail-closed.
        const PREFIX: &[u8] =
            b"onnxruntime cpuid_info warning: Unknown CPU vendor. cpuinfo_vendor value: ";
        let Some(after_prefix) = output.strip_prefix(PREFIX) else {
            return output;
        };
        let line_end = after_prefix
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap_or(after_prefix.len());
        let vendor_line = &after_prefix[..line_end];
        let vendor = vendor_line.strip_suffix(b"\r").unwrap_or(vendor_line);
        if vendor.is_empty() || !vendor.iter().all(u8::is_ascii_digit) {
            return output;
        }
        if line_end == after_prefix.len() {
            Vec::new()
        } else {
            after_prefix[line_end + 1..].to_vec()
        }
    }
    #[cfg(not(target_os = "linux"))]
    output
}

/// Spawn one strictly typed read-only Git command with `target` installed as
/// the helper's cwd via `fchdir`. The caller owns output/deadline enforcement.
#[cfg(target_os = "linux")]
pub(crate) fn spawn_pinned_read_git_helper(
    target: &fs::File,
    expected_target_path: &Path,
    _git_executable: &Path,
    launch: &GitLaunchAuthority,
    command: CodePinnedReadCommand,
    disabled_filter_keys: Vec<String>,
) -> Result<Child, String> {
    let request = PinnedGitRequest::ReadOnly {
        git_executable: path_to_string(launch.path(), "pinned read-only Git executable")?,
        command,
        disabled_filter_keys,
        expected_target_path: path_to_string(expected_target_path, "pinned read-only Git root")?,
    };
    validate_pinned_git_envelope(&PinnedGitEnvelope {
        version: PINNED_GIT_REQUEST_VERSION,
        target_device: 0,
        target_inode: 0,
        request: clone_pinned_git_request(&request),
    })?;
    spawn_pinned_git_direct_child(&request, target, launch)
}

#[cfg(all(target_os = "macos", not(test)))]
pub(crate) fn spawn_pinned_read_git_helper(
    target: &fs::File,
    expected_target_path: &Path,
    _git_executable: &Path,
    session: &MacGitAuthoritySession,
    command: CodePinnedReadCommand,
    disabled_filter_keys: Vec<String>,
) -> Result<PinnedGitChild, String> {
    let request = PinnedGitRequest::ReadOnly {
        git_executable: MACOS_SYSTEM_GIT.to_string(),
        command,
        disabled_filter_keys,
        expected_target_path: path_to_string(expected_target_path, "pinned read-only Git root")?,
    };
    validate_pinned_git_envelope(&PinnedGitEnvelope {
        version: PINNED_GIT_REQUEST_VERSION,
        target_device: 0,
        target_inode: 0,
        request: clone_pinned_git_request(&request),
    })?;
    spawn_pinned_git_macos_child(&request, target, session)
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos")), not(test)))]
pub(crate) fn spawn_pinned_read_git_helper(
    _target: &fs::File,
    _expected_target_path: &Path,
    _git_executable: &Path,
    _command: CodePinnedReadCommand,
    _disabled_filter_keys: Vec<String>,
) -> Result<PinnedGitChild, String> {
    Err("pinned Git launch is unsupported on this Unix platform".to_string())
}

#[cfg(all(target_os = "macos", test))]
pub(crate) fn spawn_pinned_read_git_helper(
    target: &fs::File,
    expected_target_path: &Path,
    git_executable: &Path,
    command: CodePinnedReadCommand,
    disabled_filter_keys: Vec<String>,
) -> Result<PinnedGitChild, String> {
    let request = PinnedGitRequest::ReadOnly {
        git_executable: path_to_string(git_executable, "pinned read-only Git executable")?,
        command,
        disabled_filter_keys,
        expected_target_path: path_to_string(expected_target_path, "pinned read-only Git root")?,
    };
    spawn_pinned_git_path_helper_child(&request, target)
}

#[cfg(target_os = "linux")]
fn spawn_pinned_git_direct_child(
    request: &PinnedGitRequest,
    target: &fs::File,
    authority: &GitLaunchAuthority,
) -> Result<Child, String> {
    use std::os::fd::AsFd as _;
    use std::os::unix::fs::MetadataExt as _;

    let metadata = target
        .metadata()
        .map_err(|error| format!("failed to inspect pinned Git target: {error}"))?;
    let envelope = PinnedGitEnvelope {
        version: PINNED_GIT_REQUEST_VERSION,
        target_device: metadata.dev(),
        target_inode: metadata.ino(),
        request: clone_pinned_git_request(request),
    };
    validate_pinned_git_envelope(&envelope)?;
    let encoded = serde_json::to_vec(&envelope)
        .map_err(|error| format!("failed to encode pinned Git request: {error}"))?;
    if encoded.len() > MAX_PINNED_GIT_REQUEST_BYTES {
        return Err(format!(
            "pinned Git request exceeded {MAX_PINNED_GIT_REQUEST_BYTES} bytes"
        ));
    }
    if request.git_executable() != authority.path() {
        return Err(
            "pinned Git request did not match its root-trusted launch authority".to_string(),
        );
    }
    let pinned_untracked_file = match request {
        PinnedGitRequest::ReadOnly {
            command: CodePinnedReadCommand::UntrackedPatch { path },
            ..
        } => Some(open_pinned_untracked_file(target.as_fd(), path)?),
        _ => None,
    };
    let mut command = authority.command();
    configure_pinned_git_command(command.command_mut(), request)?;
    authority.spawn(
        target,
        command,
        pinned_untracked_file
            .map(Stdio::from)
            .unwrap_or_else(Stdio::null),
    )
}

#[cfg(all(target_os = "macos", not(test)))]
fn spawn_pinned_git_macos_child(
    request: &PinnedGitRequest,
    target: &fs::File,
    session: &MacGitAuthoritySession,
) -> Result<MacGitChild, String> {
    use std::os::fd::AsFd as _;
    use std::os::unix::fs::MetadataExt as _;

    let metadata = target
        .metadata()
        .map_err(|error| format!("failed to inspect pinned Git target: {error}"))?;
    let envelope = PinnedGitEnvelope {
        version: PINNED_GIT_REQUEST_VERSION,
        target_device: metadata.dev(),
        target_inode: metadata.ino(),
        request: clone_pinned_git_request(request),
    };
    validate_pinned_git_envelope(&envelope)?;
    if request.git_executable() != Path::new(MACOS_SYSTEM_GIT) {
        return Err(
            "pinned Git request did not match the macOS root-trusted launch authority".to_string(),
        );
    }
    let encoded = serde_json::to_string(&envelope)
        .map_err(|error| format!("failed to encode pinned Git request: {error}"))?;
    if encoded.len() > MAX_PINNED_GIT_REQUEST_BYTES {
        return Err(format!(
            "pinned Git request exceeded {MAX_PINNED_GIT_REQUEST_BYTES} bytes"
        ));
    }
    let input = match request {
        PinnedGitRequest::ReadOnly {
            command: CodePinnedReadCommand::UntrackedPatch { path },
            ..
        } => MacGitInput::File(open_pinned_untracked_file(target.as_fd(), path)?),
        _ => MacGitInput::Null,
    };
    session.spawn(MacGitFamily::Pinned, encoded, target, input)
}

#[cfg(all(unix, not(target_os = "linux"), test))]
fn spawn_pinned_git_path_helper_child(
    request: &PinnedGitRequest,
    target: &fs::File,
) -> Result<Child, String> {
    use std::os::unix::fs::MetadataExt;

    let metadata = target
        .metadata()
        .map_err(|error| format!("failed to inspect pinned Git target: {error}"))?;
    let envelope = PinnedGitEnvelope {
        version: PINNED_GIT_REQUEST_VERSION,
        target_device: metadata.dev(),
        target_inode: metadata.ino(),
        request: clone_pinned_git_request(request),
    };
    let encoded = serde_json::to_string(&envelope)
        .map_err(|error| format!("failed to encode pinned Git request: {error}"))?;
    if encoded.len() > MAX_PINNED_GIT_REQUEST_BYTES {
        return Err(format!(
            "pinned Git request exceeded {MAX_PINNED_GIT_REQUEST_BYTES} bytes"
        ));
    }
    let executable = std::env::current_exe()
        .map_err(|error| format!("failed to resolve SchoolX desktop executable: {error}"))?;
    let mut command = Command::new(executable);
    command.args([
        "--exact",
        "code_workspace::worktrees::tests::pinned_git_helper_subprocess_entry",
        "--ignored",
        "--nocapture",
    ]);
    command
        .env(PINNED_GIT_REQUEST_ENV, encoded)
        .stdin(Stdio::from(target.try_clone().map_err(|error| {
            format!("failed to clone pinned Git target: {error}")
        })?))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::util::configure_no_window(&mut command);
    use std::os::unix::process::CommandExt;
    command.process_group(0);
    let child = command
        .spawn()
        .map_err(|error| format!("failed to start pinned Git helper: {error}"))?;
    Ok(child)
}

#[cfg(unix)]
fn clone_pinned_git_request(request: &PinnedGitRequest) -> PinnedGitRequest {
    match request {
        PinnedGitRequest::WorktreeAdd {
            git_executable,
            git_common_dir,
            base_commit,
            disabled_filter_keys,
            expected_target_path,
        } => PinnedGitRequest::WorktreeAdd {
            git_executable: git_executable.clone(),
            git_common_dir: git_common_dir.clone(),
            base_commit: base_commit.clone(),
            disabled_filter_keys: disabled_filter_keys.clone(),
            expected_target_path: expected_target_path.clone(),
        },
        PinnedGitRequest::Checkout {
            git_executable,
            base_commit,
            disabled_filter_keys,
            expected_target_path,
        } => PinnedGitRequest::Checkout {
            git_executable: git_executable.clone(),
            base_commit: base_commit.clone(),
            disabled_filter_keys: disabled_filter_keys.clone(),
            expected_target_path: expected_target_path.clone(),
        },
        PinnedGitRequest::ReadOnly {
            git_executable,
            command,
            disabled_filter_keys,
            expected_target_path,
        } => PinnedGitRequest::ReadOnly {
            git_executable: git_executable.clone(),
            command: command.clone(),
            disabled_filter_keys: disabled_filter_keys.clone(),
            expected_target_path: expected_target_path.clone(),
        },
    }
}

#[cfg(unix)]
fn verify_pinned_target_chain(
    request: &PinnedGitRequest,
    directories: &[fs::File],
) -> Result<(), String> {
    let expected_target = request.expected_target_path();
    if !expected_target.is_absolute() {
        return Err("pinned Git target path must be absolute".to_string());
    }
    if matches!(request, PinnedGitRequest::ReadOnly { .. }) {
        if directories.len() != 1 {
            return Err(
                "pinned read-only Git operation did not carry its exact target handle".to_string(),
            );
        }
        return verify_named_pinned_directory(
            directories
                .first()
                .ok_or_else(|| "pinned read-only Git target handle was missing".to_string())?,
            expected_target,
        );
    }
    if directories.len() != 4 {
        return Err("pinned Git operation did not carry its complete nest chain".to_string());
    }
    let worktree_id = expected_target
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "pinned Git target had no valid worktree id".to_string())?;
    validate_worktree_id(worktree_id)?;
    let repository_identity = expected_target
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        .ok_or_else(|| "pinned Git target had no repository identity".to_string())?;
    validate_repository_identity(repository_identity)?;
    if expected_target
        .parent()
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|value| value.to_str())
        != Some(WORKTREES_DIRECTORY)
    {
        return Err("pinned Git target was outside the WORKTREES boundary".to_string());
    }

    for (index, directory) in directories.iter().enumerate() {
        let path = match index {
            0 => expected_target
                .parent()
                .and_then(Path::parent)
                .and_then(Path::parent),
            1 => expected_target.parent().and_then(Path::parent),
            2 => expected_target.parent(),
            3 => Some(expected_target),
            _ => None,
        }
        .ok_or_else(|| "pinned Git target chain was incomplete".to_string())?;
        verify_named_pinned_directory(directory, path)?;
    }
    Ok(())
}

#[cfg(unix)]
fn verify_named_pinned_directory(directory: &fs::File, path: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;

    let metadata = directory
        .metadata()
        .map_err(|error| format!("failed to inspect pinned Git directory: {error}"))?;
    if !metadata.is_dir() {
        return Err("pinned Git operation contained a non-directory handle".to_string());
    }
    let named = path.symlink_metadata().map_err(|error| {
        format!(
            "failed to verify named pinned Git directory {}: {error}",
            path.display()
        )
    })?;
    if named.file_type().is_symlink()
        || !named.is_dir()
        || named.dev() != metadata.dev()
        || named.ino() != metadata.ino()
    {
        return Err(format!(
            "pinned Git directory {} moved or was replaced",
            path.display()
        ));
    }
    Ok(())
}

#[cfg(all(unix, test))]
fn validate_helper_git_executable(value: &str) -> Result<PathBuf, String> {
    let path = Path::new(value);
    if !path.is_absolute() {
        return Err("pinned Git executable must be absolute".to_string());
    }
    let canonical = path
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize pinned Git executable: {error}"))?;
    if canonical != path || !canonical.is_file() {
        return Err("pinned Git executable is not a canonical regular file".to_string());
    }
    Ok(canonical)
}

#[cfg(unix)]
fn validate_filter_override_keys(keys: &[String]) -> Result<(), String> {
    if keys.len() > MAX_PINNED_GIT_FILTER_KEYS {
        return Err(format!(
            "pinned Git filter overrides exceeded the {MAX_PINNED_GIT_FILTER_KEYS}-key limit"
        ));
    }
    for key in keys {
        let normalized = key.to_ascii_lowercase();
        let valid_suffix = [".clean", ".smudge", ".process", ".required"]
            .iter()
            .any(|suffix| normalized.ends_with(suffix));
        if key.len() > 4096
            || key.chars().any(char::is_control)
            || !normalized.starts_with("filter.")
            || !valid_suffix
        {
            return Err("pinned Git filter override was invalid".to_string());
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn run_git_executable_with_filter_overrides(
    executable: &Path,
    cwd: &Path,
    args: &[OsString],
    operation: GitOperation,
    disabled_filter_keys: &[String],
    deadline: Option<Instant>,
) -> Result<Vec<u8>, String> {
    let mut command = Command::new(executable);
    command.arg("--no-pager").args(args).current_dir(cwd);
    configure_git_environment(&mut command, operation, disabled_filter_keys);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::util::configure_no_window(&mut command);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }

    let timeout = remaining_git_timeout(deadline)?;
    let child = command
        .spawn()
        .map_err(|error| format!("failed to start git: {error}"))?;
    capture_child(child, "git", timeout)
}

fn remaining_git_timeout(deadline: Option<Instant>) -> Result<Duration, String> {
    let timeout = deadline
        .map(|deadline| deadline.saturating_duration_since(Instant::now()))
        .unwrap_or(GIT_TIMEOUT)
        .min(GIT_TIMEOUT);
    if timeout.is_zero() {
        return Err("SchoolX Code worktree inspection budget was exhausted".to_string());
    }
    Ok(timeout)
}

#[cfg(any(not(target_os = "macos"), test))]
fn capture_child(mut child: Child, label: &str, timeout: Duration) -> Result<Vec<u8>, String> {
    let captured = capture_child_status(&mut child, label, timeout)?;
    if !captured.status.success() {
        return Err(captured_child_error(label, &captured));
    }
    if captured.stdout.truncated {
        return Err(format!(
            "{label} output exceeded the {GIT_OUTPUT_LIMIT}-byte SchoolX Code limit"
        ));
    }
    Ok(captured.stdout.bytes)
}

#[cfg(any(not(target_os = "macos"), test))]
fn capture_child_status(
    child: &mut Child,
    label: &str,
    timeout: Duration,
) -> Result<CapturedChild, String> {
    let stdout_thread = spawn_pipe_reader(child.stdout.take());
    let stderr_thread = spawn_pipe_reader(child.stderr.take());
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                let _ = crate::managed_agents::terminate_process(child.id());
                let _ = child.kill();
                let _ = child.wait();
                let _ = join_pipe(stdout_thread);
                let _ = join_pipe(stderr_thread);
                return Err(format!(
                    "{label} operation timed out after {} seconds",
                    timeout.as_secs_f64()
                ));
            }
            Err(error) => {
                let _ = crate::managed_agents::terminate_process(child.id());
                let _ = child.kill();
                let _ = child.wait();
                let _ = join_pipe(stdout_thread);
                let _ = join_pipe(stderr_thread);
                return Err(format!("failed to wait for {label}: {error}"));
            }
        }
    };

    Ok(CapturedChild {
        status,
        stdout: join_pipe(stdout_thread)?,
        stderr: join_pipe(stderr_thread)?,
    })
}

#[cfg(all(target_os = "macos", not(test)))]
fn capture_macos_pinned_child(
    mut child: MacGitChild,
    label: &str,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let captured = capture_macos_pinned_child_status(&mut child, label, timeout)?;
    if !captured.status.success() {
        return Err(captured_child_error(label, &captured));
    }
    if captured.stdout.truncated {
        return Err(format!(
            "{label} output exceeded the {GIT_OUTPUT_LIMIT}-byte SchoolX Code limit"
        ));
    }
    Ok(captured.stdout.bytes)
}

#[cfg(all(target_os = "macos", not(test)))]
fn capture_macos_pinned_child_status(
    child: &mut MacGitChild,
    label: &str,
    timeout: Duration,
) -> Result<CapturedChild, String> {
    let stdout_thread = spawn_pipe_reader(Some(child.take_stdout()?));
    let stderr_thread = spawn_pipe_reader(Some(child.take_stderr()?));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Ok(None) => {
                let error = format!(
                    "{label} operation timed out after {} seconds",
                    timeout.as_secs_f64()
                );
                return Err(finish_failed_macos_capture(
                    child,
                    stdout_thread,
                    stderr_thread,
                    error,
                ));
            }
            Err(error) => {
                return Err(finish_failed_macos_capture(
                    child,
                    stdout_thread,
                    stderr_thread,
                    format!("failed to wait for {label}: {error}"),
                ));
            }
        }
    };
    Ok(CapturedChild {
        status,
        stdout: join_pipe(stdout_thread)?,
        stderr: join_pipe(stderr_thread)?,
    })
}

#[cfg(all(target_os = "macos", not(test)))]
fn finish_failed_macos_capture(
    child: &mut MacGitChild,
    stdout_thread: JoinHandle<CapturedPipe>,
    stderr_thread: JoinHandle<CapturedPipe>,
    primary_error: String,
) -> String {
    finish_failed_capture_threads(stdout_thread, stderr_thread, primary_error, || {
        child.terminate()
    })
}

#[cfg(any(all(target_os = "macos", not(test)), test))]
fn finish_failed_capture_threads<F>(
    stdout_thread: JoinHandle<CapturedPipe>,
    stderr_thread: JoinHandle<CapturedPipe>,
    primary_error: String,
    terminate: F,
) -> String
where
    F: FnOnce() -> Result<(), String>,
{
    match terminate() {
        Ok(()) => {
            // A successful cancellation is the signed service's proof that
            // the child was killed, reaped, and its process group vanished.
            // Only that proof makes waiting for both pipe EOFs safe.
            let stdout_error = join_pipe(stdout_thread).err();
            let stderr_error = join_pipe(stderr_thread).err();
            [Some(primary_error), stdout_error, stderr_error]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join("; ")
        }
        Err(termination_error) => {
            // An invalidated helper or ambiguous cleanup may leave a writer
            // alive indefinitely. Dropping JoinHandles detaches the readers;
            // session poisoning retains authority and this call stays bounded.
            drop(stdout_thread);
            drop(stderr_thread);
            format!("{primary_error}; {termination_error}")
        }
    }
}

fn captured_child_error(label: &str, captured: &CapturedChild) -> String {
    let message = String::from_utf8_lossy(&captured.stderr.bytes);
    let message = message.trim();
    let suffix = if captured.stderr.truncated {
        " [output truncated]"
    } else {
        ""
    };
    if message.is_empty() {
        format!("{label} exited with status {}{suffix}", captured.status)
    } else {
        format!("{message}{suffix}")
    }
}

fn configure_git_environment(
    command: &mut Command,
    operation: GitOperation,
    disabled_filter_keys: &[String],
) {
    let inherited = [
        "PATH",
        "TMPDIR",
        "TMP",
        "TEMP",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
    ]
    .into_iter()
    .filter_map(|name| std::env::var_os(name).map(|value| (name, value)))
    .collect::<Vec<_>>();
    command.env_clear();
    for (name, value) in inherited {
        command.env(name, value);
    }
    command.env("GIT_NO_REPLACE_OBJECTS", "1");
    command.env("GIT_NO_LAZY_FETCH", "1");
    #[cfg(unix)]
    command.env("GIT_GRAFT_FILE", "/dev/null");
    command.env("GIT_TERMINAL_PROMPT", "0");
    command.env("GIT_CONFIG_NOSYSTEM", "1");
    command.env("GIT_CONFIG_SYSTEM", "/dev/null");
    command.env("GIT_CONFIG_GLOBAL", "/dev/null");
    command.env("GIT_ATTR_NOSYSTEM", "1");
    command.env(
        "GIT_OPTIONAL_LOCKS",
        match operation {
            GitOperation::ReadOnly | GitOperation::WorkingTreeRead => "0",
            GitOperation::Mutating => "1",
        },
    );

    let config = [
        ("credential.helper", ""),
        ("advice.graftFileDeprecated", "false"),
        ("core.hooksPath", "/dev/null"),
        ("core.fsmonitor", "false"),
        ("protocol.allow", "never"),
    ];
    let static_config_len = config.len();
    command.env(
        "GIT_CONFIG_COUNT",
        (static_config_len + disabled_filter_keys.len()).to_string(),
    );
    for (index, (key, value)) in config.into_iter().enumerate() {
        command.env(format!("GIT_CONFIG_KEY_{index}"), key);
        command.env(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
    for (offset, key) in disabled_filter_keys.iter().enumerate() {
        let index = static_config_len + offset;
        command.env(format!("GIT_CONFIG_KEY_{index}"), key);
        command.env(
            format!("GIT_CONFIG_VALUE_{index}"),
            if key.ends_with(".required") {
                "false"
            } else {
                ""
            },
        );
    }
}

fn spawn_pipe_reader<R>(pipe: Option<R>) -> JoinHandle<CapturedPipe>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || read_pipe_capped(pipe))
}

fn read_pipe_capped<R>(pipe: Option<R>) -> CapturedPipe
where
    R: Read,
{
    let Some(mut pipe) = pipe else {
        return CapturedPipe {
            bytes: Vec::new(),
            truncated: false,
        };
    };
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    let mut truncated = false;
    loop {
        let read = match pipe.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let remaining = GIT_OUTPUT_LIMIT.saturating_sub(bytes.len());
        let retained = remaining.min(read);
        bytes.extend_from_slice(&buffer[..retained]);
        truncated |= retained < read;
    }
    CapturedPipe { bytes, truncated }
}

fn join_pipe(handle: JoinHandle<CapturedPipe>) -> Result<CapturedPipe, String> {
    handle
        .join()
        .map_err(|_| "git output reader stopped unexpectedly".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ambiguous_macos_capture_cleanup_detaches_live_pipe_readers() -> Result<(), String> {
        let (stdout_reader, stdout_writer) = std::io::pipe().map_err(|error| error.to_string())?;
        let (stderr_reader, stderr_writer) = std::io::pipe().map_err(|error| error.to_string())?;
        let stdout_thread = spawn_pipe_reader(Some(stdout_reader));
        let stderr_thread = spawn_pipe_reader(Some(stderr_reader));
        let started = Instant::now();
        let error = finish_failed_capture_threads(
            stdout_thread,
            stderr_thread,
            "poll failed".to_string(),
            || Err("cleanup disposition ambiguous".to_string()),
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(error, "poll failed; cleanup disposition ambiguous");
        drop(stdout_writer);
        drop(stderr_writer);
        Ok(())
    }

    #[test]
    fn legacy_path_git_launcher_is_test_or_non_unix_only() {
        let source = include_str!("worktrees.rs");
        assert!(source.contains("#[cfg(not(unix))]\nfn run_git_until("));
        assert!(source.contains("#[cfg(not(unix))]\nfn run_git_with_filter_overrides_until("));
        assert!(source.contains("#[cfg(any(not(unix), test))]\nfn git_executable("));
        assert!(source.contains("#[cfg(not(unix))]\nfn run_git_executable_with_filter_overrides("));
    }

    #[cfg(unix)]
    #[test]
    fn read_only_pin_uses_one_exact_named_target_descriptor() -> Result<(), String> {
        let sandbox = tempfile::tempdir().map_err(|error| error.to_string())?;
        let target = sandbox.path().join("selected-repository");
        fs::create_dir(&target).map_err(|error| error.to_string())?;
        let target = target.canonicalize().map_err(|error| error.to_string())?;
        let request = PinnedGitRequest::ReadOnly {
            git_executable: path_to_string(&git_executable()?, "test Git executable")?,
            command: CodePinnedReadCommand::TopLevel,
            disabled_filter_keys: Vec::new(),
            expected_target_path: path_to_string(&target, "test read-only target")?,
        };
        let directories = pin_pinned_git_directories(&target, &request)?;
        assert_eq!(directories.len(), 1);
        verify_pinned_target_chain(&request, &directories)?;

        let moved = sandbox.path().join("moved-selected-repository");
        fs::rename(&target, &moved).map_err(|error| error.to_string())?;
        fs::create_dir(&target).map_err(|error| error.to_string())?;
        assert!(verify_pinned_target_chain(&request, &directories)
            .is_err_and(|error| error.contains("moved or was replaced")));
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_xpc_prepare_revalidates_pinned_git_descriptors() -> Result<(), String> {
        use std::os::unix::fs::MetadataExt as _;

        if rustix::process::geteuid().as_raw() == 0 {
            return Ok(());
        }
        let target = tempfile::tempdir().map_err(|error| error.to_string())?;
        let metadata = target
            .path()
            .metadata()
            .map_err(|error| error.to_string())?;
        let envelope = PinnedGitEnvelope {
            version: PINNED_GIT_REQUEST_VERSION,
            target_device: metadata.dev(),
            target_inode: metadata.ino(),
            request: PinnedGitRequest::ReadOnly {
                git_executable: MACOS_SYSTEM_GIT.to_string(),
                command: CodePinnedReadCommand::TopLevel,
                disabled_filter_keys: Vec::new(),
                expected_target_path: path_to_string(target.path(), "test target")?,
            },
        };
        let payload = serde_json::to_string(&envelope).map_err(|error| error.to_string())?;
        let cwd = DescriptorObservation {
            device: metadata.dev(),
            inode: metadata.ino(),
            mode: metadata.mode(),
            size: 0,
        };
        let null = fs::metadata("/dev/null").map_err(|error| error.to_string())?;
        let stdin = DescriptorObservation {
            device: null.dev(),
            inode: null.ino(),
            mode: null.mode(),
            size: null.size(),
        };
        prepare_macos_pinned_git(&payload, cwd, stdin)?;
        assert!(prepare_macos_pinned_git(
            &payload,
            DescriptorObservation {
                inode: cwd.inode.saturating_add(1),
                ..cwd
            },
            stdin,
        )
        .is_err_and(|error| error.contains("descriptor identity")));
        Ok(())
    }

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

    struct TestRepository {
        _directory: tempfile::TempDir,
        root: PathBuf,
    }

    #[test]
    fn pinned_change_inventory_commands_are_literal_closed_and_non_renaming() {
        let base_commit = "a".repeat(40);
        let name_status = pinned_read_arguments(&CodePinnedReadCommand::TrackedNameStatus {
            base_commit: base_commit.clone(),
        });
        let name_status = name_status
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            name_status,
            vec![
                "--literal-pathspecs",
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--name-status",
                "-z",
                &base_commit,
                "--",
            ]
        );

        let unmerged = pinned_read_arguments(&CodePinnedReadCommand::TrackedUnmergedPaths)
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            unmerged,
            vec![
                "--literal-pathspecs",
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-renames",
                "--name-only",
                "--diff-filter=U",
                "-z",
                "--",
            ]
        );

        let untracked = pinned_read_arguments(&CodePinnedReadCommand::UntrackedPatch {
            path: "untracked.txt".to_string(),
        });
        let untracked = untracked
            .iter()
            .map(|value| value.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(untracked.iter().any(|argument| argument == "--patch"));
        assert!(!untracked.iter().any(|argument| argument == "--binary"));
        assert!(!untracked.iter().any(|argument| argument == "--numstat"));
    }

    #[cfg(unix)]
    #[test]
    fn merge_proof_commands_and_environment_are_literal_and_closed() {
        let head_commit = "a".repeat(40);
        let target_commit = "b".repeat(40);
        let merge_base = pinned_read_arguments(&CodePinnedReadCommand::MergeBaseIsAncestor {
            head_commit: head_commit.clone(),
            target_commit: target_commit.clone(),
        })
        .into_iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
        assert_eq!(
            merge_base,
            vec![
                "merge-base",
                "--is-ancestor",
                "--end-of-options",
                &head_commit,
                &target_commit,
            ]
        );

        let target_ref = "refs/heads/main".to_string();
        let direct_ref = pinned_read_arguments(&CodePinnedReadCommand::DirectLocalRefCommit {
            target_ref: target_ref.clone(),
        })
        .into_iter()
        .map(|value| value.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
        assert_eq!(
            direct_ref,
            vec![
                "rev-parse",
                "--verify",
                "--quiet",
                "--end-of-options",
                &format!("{target_ref}^{{commit}}"),
            ]
        );

        assert!(
            validate_pinned_read_command(&CodePinnedReadCommand::DirectLocalRefCommit {
                target_ref: "refs/remotes/origin/main".to_string(),
            })
            .is_err()
        );
        assert!(
            validate_pinned_read_command(&CodePinnedReadCommand::MergeBaseIsAncestor {
                head_commit: "HEAD".to_string(),
                target_commit,
            })
            .is_err()
        );

        let mut command = Command::new("git");
        configure_git_environment(&mut command, GitOperation::ReadOnly, &[]);
        let environment = command
            .get_envs()
            .filter_map(|(key, value)| {
                value.map(|value| {
                    (
                        key.to_string_lossy().into_owned(),
                        value.to_string_lossy().into_owned(),
                    )
                })
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(
            environment
                .get("GIT_NO_REPLACE_OBJECTS")
                .map(String::as_str),
            Some("1")
        );
        assert_eq!(
            environment.get("GIT_NO_LAZY_FETCH").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            environment.get("GIT_GRAFT_FILE").map(String::as_str),
            Some("/dev/null")
        );
        assert_eq!(
            environment.get("GIT_OPTIONAL_LOCKS").map(String::as_str),
            Some("0")
        );
        let configured_keys = environment
            .iter()
            .filter_map(|(key, value)| key.starts_with("GIT_CONFIG_KEY_").then_some(value.as_str()))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            configured_keys,
            [
                "advice.graftFileDeprecated",
                "core.fsmonitor",
                "core.hooksPath",
                "credential.helper",
                "protocol.allow",
            ]
            .into_iter()
            .collect::<BTreeSet<_>>()
        );
    }

    fn test_git(cwd: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
        let executable = crate::managed_agents::resolve_command("git")
            .ok_or_else(|| "git executable was not found".to_string())?;
        let output = Command::new(executable)
            .arg("--no-pager")
            .args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .map_err(|error| format!("failed to run test git: {error}"))?;
        if output.status.success() {
            Ok(output.stdout)
        } else {
            Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
        }
    }

    fn create_repository() -> Result<TestRepository, String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let root = directory.path().join("repository");
        fs::create_dir(&root).map_err(|error| error.to_string())?;
        test_git(&root, &["init", "--initial-branch=main"])?;
        fs::write(root.join("README.md"), "first\n").map_err(|error| error.to_string())?;
        test_git(&root, &["add", "README.md"])?;
        test_git(
            &root,
            &[
                "-c",
                "user.name=SchoolX Test",
                "-c",
                "user.email=schoolx@example.invalid",
                "commit",
                "-m",
                "initial",
            ],
        )?;
        Ok(TestRepository {
            _directory: directory,
            root,
        })
    }

    fn test_line(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).trim().to_string()
    }

    fn test_commit_file(
        repository_root: &Path,
        path: &str,
        contents: &str,
        message: &str,
    ) -> Result<String, String> {
        fs::write(repository_root.join(path), contents).map_err(|error| error.to_string())?;
        test_git(repository_root, &["add", "--", path])?;
        test_git(
            repository_root,
            &[
                "-c",
                "user.name=SchoolX Test",
                "-c",
                "user.email=schoolx@example.invalid",
                "commit",
                "-m",
                message,
            ],
        )?;
        Ok(test_line(&test_git(
            repository_root,
            &["rev-parse", "HEAD"],
        )?))
    }

    fn proof_observation(source_root: &Path, managed_root: &Path) -> Result<Vec<Vec<u8>>, String> {
        Ok(vec![
            test_git(
                source_root,
                &["for-each-ref", "--format=%(refname) %(objectname)"],
            )?,
            test_git(managed_root, &["rev-parse", "HEAD"])?,
            test_git(managed_root, &["status", "--porcelain=v1", "-z"])?,
            fs::read(managed_root.join(".git")).map_err(|error| error.to_string())?,
        ])
    }

    #[test]
    fn prepares_detached_head_without_mutating_the_original_checkout() -> Result<(), String> {
        let repository = create_repository()?;
        let original_head = test_line(&test_git(&repository.root, &["rev-parse", "HEAD"])?);
        let original_branch =
            test_line(&test_git(&repository.root, &["branch", "--show-current"])?);
        let original_status = test_git(&repository.root, &["status", "--porcelain=v1", "-z"])?;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;

        let prepared = prepare_execution_root(
            CodeWorktreePrepareInput {
                repository_root: path_to_string(&repository.root, "test repository")?,
                base_ref: "HEAD".to_string(),
                execution_mode: CodeExecutionMode::Worktree,
            },
            nest.path(),
        )?;

        assert_eq!(prepared.descriptor.base_ref, original_head);
        assert_eq!(prepared.head_commit, original_head);
        assert_eq!(prepared.branch, None);
        assert!(!prepared.dirty);
        let worktree_id = prepared
            .descriptor
            .worktree_id
            .as_deref()
            .ok_or_else(|| "expected a managed worktree id".to_string())?;
        let expected_root = nest
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?
            .join(WORKTREES_DIRECTORY)
            .join(&prepared.descriptor.repository_identity)
            .join(worktree_id);
        assert_eq!(
            Path::new(&prepared.descriptor.execution_root),
            expected_root
        );
        assert!(test_git(&expected_root, &["symbolic-ref", "-q", "HEAD"]).is_err());
        assert_eq!(
            test_line(&test_git(&repository.root, &["rev-parse", "HEAD"])?),
            original_head
        );
        assert_eq!(
            test_line(&test_git(&repository.root, &["branch", "--show-current"])?),
            original_branch
        );
        assert_eq!(
            test_git(&repository.root, &["status", "--porcelain=v1", "-z"])?,
            original_status
        );
        assert_eq!(
            revalidate_execution_root(&prepared.descriptor, nest.path())?.head_commit,
            original_head
        );
        Ok(())
    }

    #[test]
    fn captures_only_same_commit_direct_local_branch_authority() -> Result<(), String> {
        let repository = create_repository()?;
        let repository_info = discover_repository(&repository.root)?;
        let head = resolve_commit(&repository.root, "HEAD")?;
        assert_eq!(
            capture_direct_local_merge_target(&repository_info, "HEAD", &head)?.as_deref(),
            Some("refs/heads/main")
        );
        assert_eq!(
            capture_direct_local_merge_target(&repository_info, "main", &head)?.as_deref(),
            Some("refs/heads/main")
        );
        assert_eq!(
            capture_direct_local_merge_target(&repository_info, "refs/heads/main", &head)?
                .as_deref(),
            Some("refs/heads/main")
        );

        test_git(&repository.root, &["tag", "schoolx-tag"])?;
        test_git(
            &repository.root,
            &["update-ref", "refs/remotes/origin/main", &head],
        )?;
        for rejected in [
            "schoolx-tag",
            "refs/tags/schoolx-tag",
            "refs/remotes/origin/main",
            "origin/HEAD",
            "main~0",
            head.as_str(),
        ] {
            assert_eq!(
                capture_direct_local_merge_target(&repository_info, rejected, &head)?,
                None,
                "unexpected authority for {rejected}"
            );
        }

        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let prepared = prepare_execution_root_with_merge_target(
            CodeWorktreePrepareInput {
                repository_root: path_to_string(&repository.root, "test repository")?,
                base_ref: "HEAD".to_string(),
                execution_mode: CodeExecutionMode::Worktree,
            },
            nest.path(),
        )?;
        assert_eq!(
            prepared.merge_target_ref.as_deref(),
            Some("refs/heads/main")
        );

        test_git(&repository.root, &["checkout", "--detach", "HEAD"])?;
        let detached = discover_repository(&repository.root)?;
        assert_eq!(
            capture_direct_local_merge_target(&detached, "HEAD", &head)?,
            None
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn merge_proof_distinguishes_authorized_ancestry_and_is_zero_mutation() -> Result<(), String> {
        let repository = create_repository()?;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let prepared = prepare_execution_root_with_merge_target(
            CodeWorktreePrepareInput {
                repository_root: path_to_string(&repository.root, "test repository")?,
                base_ref: "HEAD".to_string(),
                execution_mode: CodeExecutionMode::Worktree,
            },
            nest.path(),
        )?;
        let descriptor = &prepared.worktree.descriptor;
        let target_ref = prepared
            .merge_target_ref
            .as_deref()
            .ok_or_else(|| "expected merge target".to_string())?;
        let managed_root = Path::new(&descriptor.execution_root);
        let initial = descriptor.base_ref.clone();
        let before = proof_observation(&repository.root, managed_root)?;
        let initial_proof = prove_direct_local_ancestry_before(
            descriptor,
            nest.path(),
            target_ref,
            Instant::now() + Duration::from_secs(30),
        )?;
        let CodeMergeProofOutcome::Proven(receipt) = initial_proof else {
            return Err("H == T was not proven".to_string());
        };
        assert_eq!(receipt.head_commit, initial);
        assert_eq!(receipt.target_commit, initial);
        assert_eq!(receipt.target_ref, "refs/heads/main");
        assert_eq!(receipt.repository_identity, descriptor.repository_identity);
        assert_eq!(
            receipt.worktree_id,
            descriptor.worktree_id.clone().unwrap_or_default()
        );
        assert_eq!(proof_observation(&repository.root, managed_root)?, before);

        let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
        let store = CodeThreadBindingStore::for_app_data(app_data.path())?;
        let scope = crate::code_workspace::CodeThreadBindingScope {
            community_id: "community".to_string(),
            project_dtag: "project".to_string(),
            repository_identity: descriptor.repository_identity.clone(),
        };
        let preparation_id = "11111111-1111-4111-8111-111111111111";
        store.create_preparation_with_merge_target(
            preparation_id.to_string(),
            scope.clone(),
            descriptor,
            Some(target_ref.to_string()),
        )?;
        store.claim_preparation_for_start(&scope, preparation_id, Vec::new())?;
        store.commit_preparation_binding(&scope, preparation_id, "thread-proof")?;
        let store_before = fs::read(store.store_path()).map_err(|error| error.to_string())?;
        let native = prove_binding_merge_target_before(
            &store,
            &CodeThreadBindingLookupInput {
                scope,
                codex_thread_id: "thread-proof".to_string(),
            },
            nest.path(),
            Instant::now() + Duration::from_secs(30),
        )?;
        assert!(matches!(native, Some(CodeMergeProofOutcome::Proven(_))));
        assert_eq!(
            fs::read(store.store_path()).map_err(|error| error.to_string())?,
            store_before
        );

        let task_head = test_commit_file(managed_root, "task.txt", "task\n", "task")?;
        assert_eq!(
            prove_direct_local_ancestry_before(
                descriptor,
                nest.path(),
                target_ref,
                Instant::now() + Duration::from_secs(30),
            )?,
            CodeMergeProofOutcome::NotMerged
        );
        test_git(
            &repository.root,
            &["update-ref", "refs/heads/other", &task_head],
        )?;
        assert_eq!(
            prove_direct_local_ancestry_before(
                descriptor,
                nest.path(),
                target_ref,
                Instant::now() + Duration::from_secs(30),
            )?,
            CodeMergeProofOutcome::NotMerged
        );

        test_git(
            &repository.root,
            &[
                "-c",
                "user.name=SchoolX Test",
                "-c",
                "user.email=schoolx@example.invalid",
                "merge",
                "--no-ff",
                "-m",
                "merge task",
                &task_head,
            ],
        )?;
        let merged_target = resolve_commit(&repository.root, "refs/heads/main")?;
        let before_merged_proof = proof_observation(&repository.root, managed_root)?;
        let merged = prove_direct_local_ancestry_before(
            descriptor,
            nest.path(),
            target_ref,
            Instant::now() + Duration::from_secs(30),
        )?;
        let CodeMergeProofOutcome::Proven(receipt) = merged else {
            return Err("merge commit ancestry was not proven".to_string());
        };
        assert_eq!(receipt.head_commit, task_head);
        assert_eq!(receipt.target_commit, merged_target);
        assert_eq!(
            proof_observation(&repository.root, managed_root)?,
            before_merged_proof
        );
        assert!(!repository.root.join(".git/index.lock").exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn merge_proof_rejects_squash_grafts_deadline_and_snapshot_drift() -> Result<(), String> {
        let repository = create_repository()?;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let prepared = prepare_execution_root_with_merge_target(
            CodeWorktreePrepareInput {
                repository_root: path_to_string(&repository.root, "test repository")?,
                base_ref: "HEAD".to_string(),
                execution_mode: CodeExecutionMode::Worktree,
            },
            nest.path(),
        )?;
        let descriptor = &prepared.worktree.descriptor;
        let managed_root = Path::new(&descriptor.execution_root);
        let task_head = test_commit_file(managed_root, "squash.txt", "task\n", "task")?;
        test_git(&repository.root, &["merge", "--squash", &task_head])?;
        test_git(
            &repository.root,
            &[
                "-c",
                "user.name=SchoolX Test",
                "-c",
                "user.email=schoolx@example.invalid",
                "commit",
                "-m",
                "squash task",
            ],
        )?;
        assert_eq!(
            prove_direct_local_ancestry_before(
                descriptor,
                nest.path(),
                "refs/heads/main",
                Instant::now() + Duration::from_secs(30),
            )?,
            CodeMergeProofOutcome::NotMerged
        );

        let before_deadline = proof_observation(&repository.root, managed_root)?;
        assert!(prove_direct_local_ancestry_before(
            descriptor,
            nest.path(),
            "refs/heads/main",
            Instant::now(),
        )
        .is_err());
        assert_eq!(
            proof_observation(&repository.root, managed_root)?,
            before_deadline
        );

        let previous_target = resolve_commit(&repository.root, "refs/heads/main")?;
        let drift = prove_direct_local_ancestry_with_hook(
            descriptor,
            nest.path(),
            "refs/heads/main",
            Instant::now() + Duration::from_secs(30),
            || {
                test_git(
                    &repository.root,
                    &[
                        "update-ref",
                        "refs/heads/main",
                        &task_head,
                        &previous_target,
                    ],
                )?;
                Ok(())
            },
        );
        assert!(drift.is_err());
        let head_drift = prove_direct_local_ancestry_with_hook(
            descriptor,
            nest.path(),
            "refs/heads/main",
            Instant::now() + Duration::from_secs(30),
            || {
                test_git(
                    managed_root,
                    &["update-ref", "HEAD", descriptor.base_ref.as_str()],
                )?;
                Ok(())
            },
        );
        assert!(head_drift.is_err());

        let second_repository = create_repository()?;
        let second_nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let second = prepare_execution_root_with_merge_target(
            CodeWorktreePrepareInput {
                repository_root: path_to_string(&second_repository.root, "test repository")?,
                base_ref: "HEAD".to_string(),
                execution_mode: CodeExecutionMode::Worktree,
            },
            second_nest.path(),
        )?;
        let common_dir = discover_repository(&second_repository.root)?.common_dir;
        fs::create_dir_all(common_dir.join("info")).map_err(|error| error.to_string())?;
        fs::write(common_dir.join("info/grafts"), b"forged ancestry\n")
            .map_err(|error| error.to_string())?;
        assert!(prove_direct_local_ancestry_before(
            &second.worktree.descriptor,
            second_nest.path(),
            "refs/heads/main",
            Instant::now() + Duration::from_secs(30),
        )
        .is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn merge_proof_ignores_replace_only_ancestry_and_rejects_missing_target() -> Result<(), String>
    {
        let repository = create_repository()?;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let prepared = prepare_execution_root_with_merge_target(
            CodeWorktreePrepareInput {
                repository_root: path_to_string(&repository.root, "test repository")?,
                base_ref: "HEAD".to_string(),
                execution_mode: CodeExecutionMode::Worktree,
            },
            nest.path(),
        )?;
        let descriptor = &prepared.worktree.descriptor;
        let managed_root = Path::new(&descriptor.execution_root);
        let task_head = test_commit_file(managed_root, "replace.txt", "task\n", "task")?;
        let target = resolve_commit(&repository.root, "refs/heads/main")?;
        let tree = test_line(&test_git(
            &repository.root,
            &["show", "-s", "--format=%T", &target],
        )?);
        let replacement = test_line(&test_git(
            &repository.root,
            &[
                "-c",
                "user.name=SchoolX Test",
                "-c",
                "user.email=schoolx@example.invalid",
                "commit-tree",
                &tree,
                "-p",
                &task_head,
                "-m",
                "replacement ancestry",
            ],
        )?);
        test_git(&repository.root, &["replace", &target, &replacement])?;
        assert!(test_git(
            &repository.root,
            &["merge-base", "--is-ancestor", &task_head, &target],
        )
        .is_ok());

        let before = proof_observation(&repository.root, managed_root)?;
        assert_eq!(
            prove_direct_local_ancestry_before(
                descriptor,
                nest.path(),
                "refs/heads/main",
                Instant::now() + Duration::from_secs(30),
            )?,
            CodeMergeProofOutcome::NotMerged
        );
        assert_eq!(proof_observation(&repository.root, managed_root)?, before);

        test_git(&repository.root, &["update-ref", "-d", "refs/heads/main"])?;
        assert!(prove_direct_local_ancestry_before(
            descriptor,
            nest.path(),
            "refs/heads/main",
            Instant::now() + Duration::from_secs(30),
        )
        .is_err());

        let common_dir = discover_repository(&repository.root)?.common_dir;
        let target_ref_path = common_dir.join("refs/heads/main");
        let missing_commit = "f".repeat(40);
        fs::write(&target_ref_path, format!("{missing_commit}\n"))
            .map_err(|error| error.to_string())?;
        let missing_object = common_dir
            .join("objects")
            .join(&missing_commit[..2])
            .join(&missing_commit[2..]);
        assert!(!missing_object.exists());
        let before_ref = fs::read(&target_ref_path).map_err(|error| error.to_string())?;
        let before_gitfile =
            fs::read(managed_root.join(".git")).map_err(|error| error.to_string())?;
        let before_head = test_git(managed_root, &["rev-parse", "HEAD"])?;
        assert!(prove_direct_local_ancestry_before(
            descriptor,
            nest.path(),
            "refs/heads/main",
            Instant::now() + Duration::from_secs(30),
        )
        .is_err());
        assert_eq!(
            fs::read(&target_ref_path).map_err(|error| error.to_string())?,
            before_ref
        );
        assert_eq!(
            fs::read(managed_root.join(".git")).map_err(|error| error.to_string())?,
            before_gitfile
        );
        assert_eq!(test_git(managed_root, &["rev-parse", "HEAD"])?, before_head);
        assert!(!missing_object.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn merge_proof_rejects_cherry_pick_equivalence() -> Result<(), String> {
        let repository = create_repository()?;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let prepared = prepare_execution_root_with_merge_target(
            CodeWorktreePrepareInput {
                repository_root: path_to_string(&repository.root, "test repository")?,
                base_ref: "HEAD".to_string(),
                execution_mode: CodeExecutionMode::Worktree,
            },
            nest.path(),
        )?;
        let descriptor = &prepared.worktree.descriptor;
        let managed_root = Path::new(&descriptor.execution_root);
        let task_head = test_commit_file(managed_root, "picked.txt", "task\n", "task")?;
        test_commit_file(
            &repository.root,
            "main-only.txt",
            "main\n",
            "advance target before cherry-pick",
        )?;
        test_git(
            &repository.root,
            &[
                "-c",
                "user.name=SchoolX Test",
                "-c",
                "user.email=schoolx@example.invalid",
                "cherry-pick",
                &task_head,
            ],
        )?;
        assert_eq!(
            prove_direct_local_ancestry_before(
                descriptor,
                nest.path(),
                "refs/heads/main",
                Instant::now() + Duration::from_secs(30),
            )?,
            CodeMergeProofOutcome::NotMerged
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn repository_filters_cannot_execute_during_prepare_or_status() -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt as _;

        let repository = create_repository()?;
        fs::write(repository.root.join("filtered.txt"), "first\n")
            .map_err(|error| error.to_string())?;
        fs::write(repository.root.join("worktree-only.txt"), "worktree\n")
            .map_err(|error| error.to_string())?;
        fs::write(repository.root.join("conditional.txt"), "conditional\n")
            .map_err(|error| error.to_string())?;
        fs::write(
            repository.root.join(".gitattributes"),
            concat!(
                "filtered.txt filter=schoolxevil\n",
                "worktree-only.txt filter=schoolxworktree\n",
                "conditional.txt filter=schoolxconditional\n",
            ),
        )
        .map_err(|error| error.to_string())?;
        test_git(
            &repository.root,
            &[
                "add",
                ".gitattributes",
                "filtered.txt",
                "worktree-only.txt",
                "conditional.txt",
            ],
        )?;
        test_git(
            &repository.root,
            &[
                "-c",
                "user.name=SchoolX Test",
                "-c",
                "user.email=schoolx@example.invalid",
                "commit",
                "-m",
                "filtered fixture",
            ],
        )?;

        let marker = repository.root.join("filter-executed");
        let script = repository.root.join("filter-driver.sh");
        fs::write(
            &script,
            format!("#!/bin/sh\ntouch '{}'\ncat\n", marker.display()),
        )
        .map_err(|error| error.to_string())?;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
        test_git(
            &repository.root,
            &[
                "config",
                "filter.schoolxevil.smudge",
                &script.to_string_lossy(),
            ],
        )?;
        test_git(
            &repository.root,
            &[
                "config",
                "filter.schoolxevil.clean",
                &script.to_string_lossy(),
            ],
        )?;
        test_git(
            &repository.root,
            &["config", "filter.schoolxevil.required", "true"],
        )?;
        test_git(
            &repository.root,
            &["config", "extensions.worktreeConfig", "true"],
        )?;
        test_git(
            &repository.root,
            &[
                "config",
                "--worktree",
                "filter.schoolxworktree.process",
                &script.to_string_lossy(),
            ],
        )?;
        let common_dir = test_line(&test_git(
            &repository.root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        )?);
        let conditional_config = repository.root.join("conditional-filter.conf");
        test_git(
            repository.root.as_path(),
            &[
                "config",
                "-f",
                &conditional_config.to_string_lossy(),
                "filter.schoolxconditional.process",
                &script.to_string_lossy(),
            ],
        )?;
        test_git(
            &repository.root,
            &[
                "config",
                "-f",
                &conditional_config.to_string_lossy(),
                "filter.schoolxconditional.required",
                "true",
            ],
        )?;
        let conditional_key = format!("includeIf.gitdir:{common_dir}/worktrees/**.path");
        test_git(
            &repository.root,
            &[
                "config",
                &conditional_key,
                &conditional_config.to_string_lossy(),
            ],
        )?;

        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let prepared = prepare_execution_root(
            CodeWorktreePrepareInput {
                repository_root: path_to_string(&repository.root, "test repository")?,
                base_ref: "HEAD".to_string(),
                execution_mode: CodeExecutionMode::Worktree,
            },
            nest.path(),
        )?;
        assert!(!marker.exists());

        fs::write(
            Path::new(&prepared.descriptor.execution_root).join("filtered.txt"),
            "other\n",
        )
        .map_err(|error| error.to_string())?;
        let status = revalidate_execution_root(&prepared.descriptor, nest.path())?;
        assert!(status.dirty);
        assert!(!marker.exists());
        Ok(())
    }

    #[test]
    fn resolves_a_named_ref_and_local_mode_does_not_create_the_nest() -> Result<(), String> {
        let repository = create_repository()?;
        let tagged_commit = test_line(&test_git(&repository.root, &["rev-parse", "HEAD"])?);
        test_git(&repository.root, &["tag", "schoolx-base"])?;
        fs::write(repository.root.join("README.md"), "second\n")
            .map_err(|error| error.to_string())?;
        test_git(&repository.root, &["add", "README.md"])?;
        test_git(
            &repository.root,
            &[
                "-c",
                "user.name=SchoolX Test",
                "-c",
                "user.email=schoolx@example.invalid",
                "commit",
                "-m",
                "second",
            ],
        )?;
        let absent_nest_parent = tempfile::tempdir().map_err(|error| error.to_string())?;
        let absent_nest = absent_nest_parent.path().join("must-not-be-created");

        let prepared = prepare_execution_root(
            CodeWorktreePrepareInput {
                repository_root: path_to_string(&repository.root, "test repository")?,
                base_ref: "schoolx-base".to_string(),
                execution_mode: CodeExecutionMode::Local,
            },
            &absent_nest,
        )?;

        assert_eq!(prepared.descriptor.base_ref, tagged_commit);
        assert!(prepared.descriptor.worktree_id.is_none());
        assert_eq!(prepared.branch.as_deref(), Some("main"));
        assert!(!absent_nest.exists());
        assert!(prepare_execution_root(
            CodeWorktreePrepareInput {
                repository_root: path_to_string(&repository.root, "test repository")?,
                base_ref: "refs/heads/does-not-exist".to_string(),
                execution_mode: CodeExecutionMode::Local,
            },
            &absent_nest,
        )
        .is_err());
        assert!(!absent_nest.exists());
        Ok(())
    }

    #[test]
    fn linked_worktrees_share_common_dir_and_repository_identity() -> Result<(), String> {
        let repository = create_repository()?;
        let linked = repository
            ._directory
            .path()
            .join("existing-linked-worktree");
        let linked_string = path_to_string(&linked, "linked test worktree")?;
        test_git(
            &repository.root,
            &["worktree", "add", "--detach", &linked_string, "HEAD"],
        )?;

        let main = inspect_repository(&path_to_string(&repository.root, "test repository")?)?;
        let linked = inspect_repository(&linked_string)?;
        assert_ne!(main.repository_root, linked.repository_root);
        assert_eq!(main.git_common_dir, linked.git_common_dir);
        assert_eq!(main.repository_identity, linked.repository_identity);
        Ok(())
    }

    #[test]
    fn repository_identity_algorithm_is_domain_separated_and_deterministic() -> Result<(), String> {
        assert_eq!(
            repository_identity(Path::new("/canonical/repo/.git"))?,
            "01b765f26c4b7868fe85614a89af89247e4cb02c20f9548200a7c32765050bbe"
        );
        Ok(())
    }

    #[test]
    fn local_status_reports_a_new_untracked_file_as_dirty() -> Result<(), String> {
        let repository = create_repository()?;
        let unused_nest = repository._directory.path().join("unused-nest");
        let prepared = prepare_execution_root(
            CodeWorktreePrepareInput {
                repository_root: path_to_string(&repository.root, "test repository")?,
                base_ref: "HEAD".to_string(),
                execution_mode: CodeExecutionMode::Local,
            },
            &unused_nest,
        )?;
        assert!(!prepared.dirty);
        assert_eq!(prepared.branch.as_deref(), Some("main"));

        fs::write(repository.root.join("untracked.txt"), "untracked\n")
            .map_err(|error| error.to_string())?;
        let status = revalidate_execution_root(&prepared.descriptor, &unused_nest)?;
        assert!(status.dirty);
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert!(!unused_nest.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn existing_managed_buckets_are_restricted_to_owner_only() -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;

        let repository = create_repository()?;
        let identity = inspect_repository(&path_to_string(&repository.root, "test repository")?)?
            .repository_identity;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let worktrees = nest.path().join(WORKTREES_DIRECTORY);
        let bucket = worktrees.join(&identity);
        fs::create_dir(&worktrees).map_err(|error| error.to_string())?;
        fs::create_dir(&bucket).map_err(|error| error.to_string())?;
        fs::set_permissions(&worktrees, fs::Permissions::from_mode(0o777))
            .map_err(|error| error.to_string())?;
        fs::set_permissions(&bucket, fs::Permissions::from_mode(0o777))
            .map_err(|error| error.to_string())?;

        prepare_execution_root(
            CodeWorktreePrepareInput {
                repository_root: path_to_string(&repository.root, "test repository")?,
                base_ref: "HEAD".to_string(),
                execution_mode: CodeExecutionMode::Worktree,
            },
            nest.path(),
        )?;

        for path in [&worktrees, &bucket] {
            let mode = fs::metadata(path)
                .map_err(|error| error.to_string())?
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o700);
        }
        Ok(())
    }

    #[test]
    fn status_rejects_a_missing_managed_target() -> Result<(), String> {
        let repository = create_repository()?;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let prepared = prepare_execution_root(
            CodeWorktreePrepareInput {
                repository_root: path_to_string(&repository.root, "test repository")?,
                base_ref: "HEAD".to_string(),
                execution_mode: CodeExecutionMode::Worktree,
            },
            nest.path(),
        )?;
        let original = PathBuf::from(&prepared.descriptor.execution_root);
        let moved = original.with_extension("moved-for-test");
        fs::rename(&original, &moved).map_err(|error| error.to_string())?;

        let error = revalidate_execution_root(&prepared.descriptor, nest.path())
            .err()
            .ok_or_else(|| "missing managed target should fail closed".to_string())?;
        assert!(error.contains("managed directory") || error.contains("inspect"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_repository_bucket_cannot_escape_the_nest() -> Result<(), String> {
        use std::os::unix::fs::symlink;

        let repository = create_repository()?;
        let identity = inspect_repository(&path_to_string(&repository.root, "test repository")?)?
            .repository_identity;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let outside = tempfile::tempdir().map_err(|error| error.to_string())?;
        let worktrees = nest.path().join(WORKTREES_DIRECTORY);
        fs::create_dir(&worktrees).map_err(|error| error.to_string())?;
        symlink(outside.path(), worktrees.join(&identity)).map_err(|error| error.to_string())?;

        let result = prepare_execution_root(
            CodeWorktreePrepareInput {
                repository_root: path_to_string(&repository.root, "test repository")?,
                base_ref: "HEAD".to_string(),
                execution_mode: CodeExecutionMode::Worktree,
            },
            nest.path(),
        );
        assert!(result.is_err());
        assert_eq!(
            fs::read_dir(outside.path())
                .map_err(|error| error.to_string())?
                .count(),
            0
        );
        Ok(())
    }

    #[test]
    fn a_non_directory_worktrees_boundary_is_rejected() -> Result<(), String> {
        let repository = create_repository()?;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        fs::write(nest.path().join(WORKTREES_DIRECTORY), "not a directory")
            .map_err(|error| error.to_string())?;

        assert!(prepare_execution_root(
            CodeWorktreePrepareInput {
                repository_root: path_to_string(&repository.root, "test repository")?,
                base_ref: "HEAD".to_string(),
                execution_mode: CodeExecutionMode::Worktree,
            },
            nest.path(),
        )
        .is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn target_reservation_creates_an_empty_real_directory_and_rejects_replacement_symlinks(
    ) -> Result<(), String> {
        use std::os::unix::fs::symlink;

        let parent = tempfile::tempdir().map_err(|error| error.to_string())?;
        let parent = parent
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let (_worktree_id, target) = reserve_worktree_target(&parent)?;

        let metadata = target
            .symlink_metadata()
            .map_err(|error| error.to_string())?;
        assert!(metadata.is_dir());
        assert!(!metadata.file_type().is_symlink());
        assert!(fs::read_dir(&target)
            .map_err(|error| error.to_string())?
            .next()
            .is_none());
        validate_reserved_worktree_target(&parent, &target)?;

        let outside = tempfile::tempdir().map_err(|error| error.to_string())?;
        fs::remove_dir(&target).map_err(|error| error.to_string())?;
        symlink(outside.path(), &target).map_err(|error| error.to_string())?;
        assert!(validate_reserved_worktree_target(&parent, &target).is_err());
        assert!(fs::read_dir(outside.path())
            .map_err(|error| error.to_string())?
            .next()
            .is_none());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn pinned_git_helper_envelope_is_bounded_and_strict() -> Result<(), String> {
        use std::os::unix::fs::MetadataExt;

        let repository = create_repository()?;
        let target = repository
            .root
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let metadata = fs::metadata(&target).map_err(|error| error.to_string())?;
        let valid = PinnedGitEnvelope {
            version: PINNED_GIT_REQUEST_VERSION,
            target_device: metadata.dev(),
            target_inode: metadata.ino(),
            request: PinnedGitRequest::Checkout {
                git_executable: path_to_string(&git_executable()?, "git executable")?,
                base_commit: resolve_commit(&repository.root, "HEAD")?,
                disabled_filter_keys: Vec::new(),
                expected_target_path: path_to_string(&target, "test target")?,
            },
        };
        validate_pinned_git_envelope(&valid)?;

        let unknown = serde_json::to_value(&valid).map_err(|error| error.to_string())?;
        let mut unknown = unknown
            .as_object()
            .cloned()
            .ok_or_else(|| "test envelope was not an object".to_string())?;
        unknown.insert("unexpected".to_string(), serde_json::json!(true));
        assert!(
            serde_json::from_value::<PinnedGitEnvelope>(serde_json::Value::Object(unknown))
                .is_err()
        );

        for forbidden_operation in [
            "remove",
            "worktreeRemove",
            "delete",
            "destroy",
            "cleanup",
            "clean",
            "prune",
            "purge",
            "discard",
        ] {
            let mut removal = serde_json::to_value(&valid).map_err(|error| error.to_string())?;
            removal["request"]["operation"] = serde_json::json!(forbidden_operation);
            assert!(
                serde_json::from_value::<PinnedGitEnvelope>(removal).is_err(),
                "current pinned Git helper accepted forbidden operation {forbidden_operation}"
            );
        }

        let too_many_filters = PinnedGitEnvelope {
            request: PinnedGitRequest::Checkout {
                git_executable: path_to_string(&git_executable()?, "git executable")?,
                base_commit: resolve_commit(&repository.root, "HEAD")?,
                disabled_filter_keys: (0..=MAX_PINNED_GIT_FILTER_KEYS)
                    .map(|index| format!("filter.test{index}.process"))
                    .collect(),
                expected_target_path: path_to_string(&target, "test target")?,
            },
            ..valid
        };
        assert!(validate_pinned_git_envelope(&too_many_filters).is_err());

        let invalid_read_filter = PinnedGitEnvelope {
            request: PinnedGitRequest::ReadOnly {
                git_executable: path_to_string(&git_executable()?, "git executable")?,
                command: CodePinnedReadCommand::LocalConfig,
                disabled_filter_keys: vec!["filter.bad.command".to_string()],
                expected_target_path: path_to_string(&target, "test target")?,
            },
            ..too_many_filters
        };
        assert!(validate_pinned_git_envelope(&invalid_read_filter).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn pinned_target_swap_cannot_redirect_git_into_an_outside_directory() -> Result<(), String> {
        use std::os::unix::fs::symlink;

        let repository = create_repository()?;
        let repository_info = discover_repository(&repository.root)?;
        let base_commit = resolve_commit(&repository.root, "HEAD")?;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let nest = nest
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let worktrees = ensure_real_child_directory(&nest, WORKTREES_DIRECTORY)?;
        let repository_bucket = ensure_real_child_directory(&worktrees, &repository_info.identity)?;
        let worktree_id = Uuid::new_v4().to_string();
        let target = repository_bucket.join(&worktree_id);
        create_private_directory(&target).map_err(|error| error.to_string())?;
        let operation = prepare_pinned_git_operation(
            &target,
            PinnedGitRequest::WorktreeAdd {
                git_executable: path_to_string(&git_executable()?, "git executable")?,
                git_common_dir: path_to_string(
                    &repository_info.common_dir,
                    "Git common directory",
                )?,
                base_commit,
                disabled_filter_keys: repository_filter_overrides(&repository.root)?,
                expected_target_path: path_to_string(&target, "managed worktree target")?,
            },
        )?;

        let outside = tempfile::tempdir().map_err(|error| error.to_string())?;
        let moved_target = outside.path().join("moved-reserved-target");
        fs::rename(&target, &moved_target).map_err(|error| error.to_string())?;
        symlink(outside.path(), &target).map_err(|error| error.to_string())?;

        assert!(run_git_with_pinned_operation(operation).is_err());
        assert!(!moved_target.join(".git").exists());
        assert!(validate_reserved_worktree_target(&repository_bucket, &target).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn helper_uses_the_pinned_directory_if_the_name_changes_after_validation() -> Result<(), String>
    {
        use std::os::unix::fs::symlink;

        let repository = create_repository()?;
        let repository_info = discover_repository(&repository.root)?;
        let base_commit = resolve_commit(&repository.root, "HEAD")?;
        let nest = tempfile::tempdir().map_err(|error| error.to_string())?;
        let nest = nest
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let worktrees = ensure_real_child_directory(&nest, WORKTREES_DIRECTORY)?;
        let repository_bucket = ensure_real_child_directory(&worktrees, &repository_info.identity)?;
        let worktree_id = Uuid::new_v4().to_string();
        let target = repository_bucket.join(&worktree_id);
        create_private_directory(&target).map_err(|error| error.to_string())?;
        let operation = prepare_pinned_git_operation(
            &target,
            PinnedGitRequest::WorktreeAdd {
                git_executable: path_to_string(&git_executable()?, "git executable")?,
                git_common_dir: path_to_string(
                    &repository_info.common_dir,
                    "Git common directory",
                )?,
                base_commit,
                disabled_filter_keys: repository_filter_overrides(&repository.root)?,
                expected_target_path: path_to_string(&target, "managed worktree target")?,
            },
        )?;
        verify_pinned_target_chain(&operation.request, &operation.directories)?;

        // Model a same-UID rename after the parent's pre-spawn validation.
        // The helper must enter the already-open target descriptor instead of
        // following the replacement symlink at the original pathname.
        let outside = tempfile::tempdir().map_err(|error| error.to_string())?;
        let moved_target = outside.path().join("moved-pinned-target");
        fs::rename(&target, &moved_target).map_err(|error| error.to_string())?;
        symlink(outside.path(), &target).map_err(|error| error.to_string())?;

        spawn_pinned_git_helper(
            &operation.request,
            &operation.directories,
            &operation.launch,
        )?;

        assert!(!outside.path().join(".git").exists());
        assert!(moved_target.join(".git").exists());
        assert!(verify_pinned_target_chain(&operation.request, &operation.directories).is_err());
        Ok(())
    }
}
