use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CStr, CString, OsStr, OsString};
use std::fs;
use std::io::{Read, Write};
#[cfg(test)]
use std::os::fd::AsFd;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
#[cfg(test)]
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
#[cfg(any(not(target_os = "macos"), test))]
use std::process::Child;
use std::process::{Command, ExitStatus, Stdio};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags, RenameFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::VerifiedRemovalAbsence;
use crate::code_workspace::bindings::removal::{
    CodeWorktreeRemovalClaimInput, CodeWorktreeRemovalRecord,
};
use crate::code_workspace::bindings::{
    validate_commit_id, validate_sha256, validate_worktree_id, CodeExecutionMode,
    CodeThreadBinding, CodeThreadBindingLookupInput, CodeThreadBindingStore,
};
#[cfg(target_os = "linux")]
use crate::code_workspace::git_launch::GitLaunchAuthority;
#[cfg(target_os = "macos")]
use crate::code_workspace::macos_git_xpc::{self, DescriptorObservation, MacGitProcessSpec};
#[cfg(all(target_os = "macos", not(test)))]
use crate::code_workspace::macos_git_xpc::{MacGitAuthoritySession, MacGitFamily, MacGitInput};
use crate::code_workspace::worktrees::{
    prove_binding_merge_target_before, repository_identity, CodeMergeProofOutcome,
    CodeMergeProofReceipt,
};

const MANIFEST_VERSION: u32 = 1;
const MANIFEST_DIRECTORY: &str = "removal-manifests-v1";
const MAX_MANIFEST_BYTES: usize = 32 * 1024 * 1024;
const MAX_MANIFEST_ENTRIES: usize = 131_072;
const MAX_GIT_OUTPUT_BYTES: usize = 32 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 1024 * 1024 * 1024;
const GIT_TIMEOUT: Duration = Duration::from_secs(60);
#[cfg(target_os = "macos")]
const MACOS_SYSTEM_GIT: &str = "/usr/bin/git";
#[cfg(test)]
const HELPER_ENV: &str = "SCHOOLX_CODE_REMOVAL_GIT_REQUEST_V1";
const HELPER_VERSION: u32 = 1;
const MAX_HELPER_REQUEST_BYTES: usize = 128 * 1024;
const MAX_FILTER_KEYS: usize = 128;
const MAX_OBJECT_TYPE_BATCH: usize = 1_024;
const MAX_OBJECT_STORAGE_ENTRIES: usize = 131_072;
const MAX_OBJECT_STORAGE_DEPTH: usize = 8;
const PROOF_REF_PREFIX: &str = "refs/schoolx/removal-claims/";
const ZERO_SHA1: &str = "0000000000000000000000000000000000000000";
const ZERO_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PhysicalRemovalManifest {
    version: u32,
    repository_identity: String,
    worktree_id: String,
    managed_root: String,
    managed_root_parent: String,
    git_admin_parent: String,
    git_admin_entry: String,
    root_parent_identity: NodeIdentity,
    common_dir_identity: NodeIdentity,
    admin_parent_identity: NodeIdentity,
    root_identity: NodeIdentity,
    admin_identity: NodeIdentity,
    root_parent_siblings: Vec<NamedIdentity>,
    admin_parent_siblings: Vec<NamedIdentity>,
    root_entries: Vec<ManifestEntry>,
    admin_entries: Vec<ManifestEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NodeIdentity {
    device: u64,
    inode: u64,
    mode: u32,
    size: u64,
    birth_time_seconds: i64,
    birth_time_nanoseconds: u32,
    generation: u64,
    content_sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct NamedIdentity {
    name_hex: String,
    identity: NodeIdentity,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
enum ManifestEntryKind {
    GitFile,
    Directory,
    RegularFile,
    Symlink,
    AdminFile,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManifestEntry {
    path_hex: String,
    kind: ManifestEntryKind,
    identity: NodeIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TrackedEntry {
    kind: TrackedKind,
    object_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TrackedKind {
    Regular { executable: bool },
    Symlink,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StoredManifest {
    digest: String,
    manifest: PhysicalRemovalManifest,
}

struct ManifestDirectory {
    handle: fs::File,
}

#[derive(Debug, Eq, PartialEq)]
struct ManifestFileRead {
    bytes: Vec<u8>,
    identity: NodeIdentity,
}

struct PinnedLayout {
    nest: fs::File,
    worktrees: fs::File,
    root_parent: fs::File,
    root: fs::File,
    common_dir: fs::File,
    admin_parent: fs::File,
    admin: fs::File,
    common_dir_path: PathBuf,
    admin_parent_path: PathBuf,
    admin_entry: OsString,
    git_launch: RemovalGitLaunchAuthority,
}

struct RecoveryBoundary {
    nest: fs::File,
    worktrees: fs::File,
    root_parent: fs::File,
    root: Option<fs::File>,
    common_dir: fs::File,
    admin_parent: fs::File,
    admin: Option<fs::File>,
    common_dir_path: PathBuf,
    git_launch: RemovalGitLaunchAuthority,
}

struct RemovalGitLaunchAuthority {
    #[cfg(target_os = "linux")]
    direct: GitLaunchAuthority,
    #[cfg(all(target_os = "macos", not(test)))]
    session: MacGitAuthoritySession,
}

impl RemovalGitLaunchAuthority {
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
                let session = MacGitAuthoritySession::begin()?;
                Ok(Self { session })
            }
            #[cfg(not(any(target_os = "linux", target_os = "macos")))]
            {
                Err("removal Git launch is unsupported on this Unix platform".to_string())
            }
            #[cfg(any(test, not(target_os = "macos")))]
            {
                Ok(Self {})
            }
        }
    }

    fn executable_string(&self) -> Result<String, String> {
        #[cfg(target_os = "linux")]
        {
            path_string(self.direct.path(), "removal root-trusted Git executable")
        }
        #[cfg(target_os = "macos")]
        {
            Ok(MACOS_SYSTEM_GIT.to_string())
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            Err("removal Git launch is unsupported on this Unix platform".to_string())
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CoordinateState {
    Absent,
    Expected,
    Replacement,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum FaultBoundary {
    Claimed,
    Removing,
    ProofRefPinned,
    Quarantined,
    RootEntryDeleted(usize),
    RootDeleted,
    AdminEntryDeleted(usize),
    AdminDeleted,
    AbsenceVerified,
    Finalized,
    ProofRefCleaned,
}

pub(super) trait FaultHook {
    fn after(&mut self, boundary: FaultBoundary) -> Result<(), String>;
}

pub(super) struct NoopFaultHook;

impl FaultHook for NoopFaultHook {
    fn after(&mut self, _boundary: FaultBoundary) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RemovalGitEnvelope {
    version: u32,
    target_device: u64,
    target_inode: u64,
    request: RemovalGitRequest,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "camelCase", deny_unknown_fields)]
enum RemovalGitRequest {
    LocalConfig {
        git_executable: String,
        expected_target_path: String,
    },
    WorktreeConfigNames {
        git_executable: String,
        expected_target_path: String,
    },
    IndexEntries {
        git_executable: String,
        expected_target_path: String,
    },
    HeadEntries {
        git_executable: String,
        expected_target_path: String,
        head_commit: String,
    },
    RefFormat {
        git_executable: String,
        expected_target_path: String,
    },
    BlobTypes {
        git_executable: String,
        expected_target_path: String,
        object_ids: Vec<String>,
    },
    Status {
        git_executable: String,
        expected_target_path: String,
        disabled_filter_keys: Vec<String>,
    },
    ReadProofRef {
        git_executable: String,
        expected_target_path: String,
        removal_id: String,
    },
    CreateProofRef {
        git_executable: String,
        expected_target_path: String,
        removal_id: String,
        target_commit: String,
        zero_oid: String,
    },
    DeleteProofRef {
        git_executable: String,
        expected_target_path: String,
        removal_id: String,
        target_commit: String,
    },
}

impl RemovalGitRequest {
    fn expected_target_path(&self) -> &Path {
        let value = match self {
            Self::LocalConfig {
                expected_target_path,
                ..
            }
            | Self::WorktreeConfigNames {
                expected_target_path,
                ..
            }
            | Self::IndexEntries {
                expected_target_path,
                ..
            }
            | Self::HeadEntries {
                expected_target_path,
                ..
            }
            | Self::BlobTypes {
                expected_target_path,
                ..
            }
            | Self::RefFormat {
                expected_target_path,
                ..
            }
            | Self::Status {
                expected_target_path,
                ..
            }
            | Self::ReadProofRef {
                expected_target_path,
                ..
            }
            | Self::CreateProofRef {
                expected_target_path,
                ..
            }
            | Self::DeleteProofRef {
                expected_target_path,
                ..
            } => expected_target_path,
        };
        Path::new(value)
    }

    fn git_executable(&self) -> &str {
        match self {
            Self::LocalConfig { git_executable, .. }
            | Self::WorktreeConfigNames { git_executable, .. }
            | Self::IndexEntries { git_executable, .. }
            | Self::HeadEntries { git_executable, .. }
            | Self::BlobTypes { git_executable, .. }
            | Self::RefFormat { git_executable, .. }
            | Self::Status { git_executable, .. }
            | Self::ReadProofRef { git_executable, .. }
            | Self::CreateProofRef { git_executable, .. }
            | Self::DeleteProofRef { git_executable, .. } => git_executable,
        }
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

fn with_removal_git_authority<T>(
    operation: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    #[cfg(all(target_os = "macos", not(test)))]
    {
        macos_git_xpc::with_authority_session(|_| {
            drop(crate::code_workspace::git_write::macos_root_trusted_git()?);
            operation()
        })
    }
    #[cfg(any(not(target_os = "macos"), test))]
    {
        operation()
    }
}

pub(super) fn claim_or_resume(
    store: &CodeThreadBindingStore,
    lookup: &CodeThreadBindingLookupInput,
    nest_root: &Path,
    deadline: Instant,
    hook: &mut dyn FaultHook,
) -> Result<CodeWorktreeRemovalRecord, String> {
    if let Some(existing) = store.lookup_worktree_removal(lookup)? {
        return with_removal_git_authority(|| {
            resume_record(store, existing, nest_root, deadline, hook)
        });
    }

    with_removal_git_authority(|| {
        let proof = exact_merge_proof(store, lookup, nest_root, deadline)?;
        let first = capture_manifest(store, lookup, &proof, nest_root, deadline)?;
        let second_proof = exact_merge_proof(store, lookup, nest_root, deadline)?;
        if proof != second_proof {
            return Err(
                "SchoolX Code removal merge proof changed during manifest capture".to_string(),
            );
        }
        let second = capture_manifest(store, lookup, &proof, nest_root, deadline)?;
        if first.manifest != second.manifest || first.digest != second.digest {
            return Err(
                "SchoolX Code physical manifest changed during claim inspection".to_string(),
            );
        }
        persist_manifest_sidecar(store, &first)?;
        let claim = claim_input(lookup, proof, &first);
        let claimed = store.get_or_claim_worktree_removal(&claim)?;
        hook.after(FaultBoundary::Claimed)?;
        resume_record(store, claimed, nest_root, deadline, hook)
    })
}

pub(super) fn recover_all(
    store: &CodeThreadBindingStore,
    nest_root: &Path,
    hook: &mut dyn FaultHook,
) -> Result<(), String> {
    for tombstone in store.list_removed_worktree_tombstones()? {
        with_removal_git_authority(|| cleanup_removed(store, &tombstone, hook))?;
    }
    for pending in store.list_pending_worktree_removals()? {
        let deadline = Instant::now() + GIT_TIMEOUT;
        with_removal_git_authority(|| {
            match resume_record(store, pending.clone(), nest_root, deadline, hook) {
                Ok(_) => Ok(()),
                Err(error) => {
                    if matches!(pending, CodeWorktreeRemovalRecord::Claimed(_)) {
                        let stored = load_manifest_sidecar(store, pending.authority())?;
                        if claimed_has_zero_mutation(&pending, &stored, nest_root, deadline)? {
                            store
                                .cancel_claimed_worktree_removal_definitely_not_started(&pending)?;
                            return Ok(());
                        }
                    }
                    Err(error)
                }
            }
        })?;
    }
    Ok(())
}

fn run_helper(
    launch: &RemovalGitLaunchAuthority,
    target: &fs::File,
    request: RemovalGitRequest,
    deadline: Instant,
) -> Result<CapturedChild, String> {
    if request.expected_target_path().as_os_str().is_empty() {
        return Err("removal Git helper target path is empty".to_string());
    }
    let metadata = target
        .metadata()
        .map_err(|error| format!("failed to inspect removal Git helper target: {error}"))?;
    if !metadata.is_dir() {
        return Err("removal Git helper target is not a directory".to_string());
    }
    let envelope = RemovalGitEnvelope {
        version: HELPER_VERSION,
        target_device: metadata.dev(),
        target_inode: metadata.ino(),
        request,
    };
    validate_helper_envelope(&envelope)?;
    let encoded = serde_json::to_string(&envelope)
        .map_err(|error| format!("failed to encode removal Git helper request: {error}"))?;
    if encoded.len() > MAX_HELPER_REQUEST_BYTES {
        return Err(format!(
            "removal Git helper request exceeds {MAX_HELPER_REQUEST_BYTES} bytes"
        ));
    }

    #[cfg(target_os = "linux")]
    let mut captured = run_direct_git(launch, target, &envelope.request, deadline)?;

    #[cfg(all(target_os = "macos", not(test)))]
    let mut captured = {
        let _ = launch;
        if envelope.request.git_executable() != MACOS_SYSTEM_GIT {
            return Err(
                "removal Git request did not match the macOS root-trusted authority".to_string(),
            );
        }
        let input = match &envelope.request {
            RemovalGitRequest::BlobTypes { object_ids, .. } => {
                let mut payload = Vec::with_capacity(object_ids.len().saturating_mul(66));
                for object_id in object_ids {
                    payload.extend_from_slice(object_id.as_bytes());
                    payload.push(b'\n');
                }
                MacGitInput::Bytes(payload)
            }
            _ => MacGitInput::Null,
        };
        let mut child = launch
            .session
            .spawn(MacGitFamily::Removal, encoded, target, input)?;
        capture_macos_removal_child(&mut child, remaining_timeout(deadline)?)?
    };

    #[cfg(all(not(target_os = "linux"), test))]
    let mut captured = {
        let _ = launch;
        let executable = std::env::current_exe()
            .map_err(|error| format!("failed to resolve removal Git helper executable: {error}"))?;
        let mut command = Command::new(executable);
        command.args([
            "--exact",
            "code_workspace::bindings::removal::physical::tests::removal_git_helper_subprocess_entry",
            "--ignored",
            "--nocapture",
        ]);
        command
            .env(HELPER_ENV, encoded)
            .stdin(Stdio::from(target.try_clone().map_err(|error| {
                format!("failed to clone removal Git helper directory: {error}")
            })?))
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        crate::util::configure_no_window(&mut command);
        command.process_group(0);
        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to start removal Git helper: {error}"))?;
        capture_child(&mut child, remaining_timeout(deadline)?)?
    };
    #[cfg(all(not(target_os = "linux"), not(target_os = "macos"), not(test)))]
    let mut captured: CapturedChild = {
        let _ = (launch, encoded);
        return Err("removal Git launch is unsupported on this Unix platform".to_string());
    };
    captured.stdout.bytes = strip_removal_test_harness_output(captured.stdout.bytes);
    captured.stderr.bytes =
        crate::code_workspace::worktrees::strip_linux_onnxruntime_startup_diagnostic(
            captured.stderr.bytes,
        );
    Ok(captured)
}

#[cfg(all(target_os = "macos", not(test)))]
fn capture_macos_removal_child(
    child: &mut crate::code_workspace::macos_git_xpc::MacGitChild,
    timeout: Duration,
) -> Result<CapturedChild, String> {
    let stdout = spawn_pipe_reader(Some(child.take_stdout()?));
    let stderr = spawn_pipe_reader(Some(child.take_stderr()?));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                return match child.terminate() {
                    Ok(()) => {
                        let _ = join_pipe(stdout);
                        let _ = join_pipe(stderr);
                        Err("SchoolX Code removal Git helper timed out".to_string())
                    }
                    Err(cleanup_error) => {
                        // The retained child may still own the pipe writers.
                        // Detach readers so an ambiguous cleanup cannot turn
                        // this bounded timeout into an unbounded join.
                        drop(stdout);
                        drop(stderr);
                        Err(format!(
                            "SchoolX Code removal Git helper timed out and child cleanup was not proven: {cleanup_error}"
                        ))
                    }
                };
            }
            Err(error) => {
                return match child.terminate() {
                    Ok(()) => {
                        let _ = join_pipe(stdout);
                        let _ = join_pipe(stderr);
                        Err(format!("failed to wait for removal Git helper: {error}"))
                    }
                    Err(cleanup_error) => {
                        drop(stdout);
                        drop(stderr);
                        Err(format!(
                            "failed to wait for removal Git helper: {error}; additionally failed to prove child cleanup: {cleanup_error}"
                        ))
                    }
                };
            }
        }
    };
    Ok(CapturedChild {
        status,
        stdout: join_pipe(stdout)?,
        stderr: join_pipe(stderr)?,
    })
}

mod delete;
mod file_identity;
mod git_helper;
mod manifest_capture;
mod manifest_store;
mod process;
mod proof_refs;
mod scan;
mod workflow;

#[cfg(test)]
pub(super) use git_helper::execute_helper;
#[cfg(target_os = "macos")]
pub(in crate::code_workspace) use git_helper::prepare_macos_removal_git;

#[cfg(target_os = "linux")]
use git_helper::run_direct_git;
use git_helper::{strip_removal_test_harness_output, validate_helper_envelope};
use manifest_capture::capture_manifest;
use manifest_store::{load_manifest_sidecar, persist_manifest_sidecar};
#[cfg(all(not(target_os = "linux"), test))]
use process::{capture_child, remaining_timeout};
#[cfg(all(target_os = "macos", not(test)))]
use process::{join_pipe, remaining_timeout, spawn_pipe_reader};
#[cfg(target_os = "linux")]
use proof_refs::path_string;
use workflow::{
    claim_input, claimed_has_zero_mutation, cleanup_removed, exact_merge_proof, resume_record,
};
