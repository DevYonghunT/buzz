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

fn claimed_has_zero_mutation(
    record: &CodeWorktreeRemovalRecord,
    stored: &StoredManifest,
    nest_root: &Path,
    deadline: Instant,
) -> Result<bool, String> {
    let authority = record.authority();
    let layout = match pin_layout(&authority.binding, nest_root) {
        Ok(layout) => layout,
        Err(_) => return Ok(false),
    };
    if verify_layout_against_authority(&layout, authority, stored).is_err() {
        return Ok(false);
    }
    let original = named_directory_state(
        &layout.root_parent,
        worktree_name(authority)?,
        &stored.manifest.root_identity,
    )?;
    let quarantine = named_directory_state(
        &layout.root_parent,
        OsStr::new(&authority.physical.quarantine_name),
        &stored.manifest.root_identity,
    )?;
    let admin = named_directory_state(
        &layout.admin_parent,
        OsStr::new(&authority.physical.git_admin_entry),
        &stored.manifest.admin_identity,
    )?;
    let proof_ref = read_proof_ref(
        &layout.git_launch,
        &layout.common_dir,
        &layout.common_dir_path,
        authority,
        deadline,
    )?;
    Ok((original, quarantine, admin)
        == (
            CoordinateState::Expected,
            CoordinateState::Absent,
            CoordinateState::Expected,
        )
        && proof_ref.is_none())
}

fn resume_record(
    store: &CodeThreadBindingStore,
    record: CodeWorktreeRemovalRecord,
    nest_root: &Path,
    deadline: Instant,
    hook: &mut dyn FaultHook,
) -> Result<CodeWorktreeRemovalRecord, String> {
    match record {
        CodeWorktreeRemovalRecord::Removed(_) => {
            cleanup_removed(store, &record, hook)?;
            Ok(record)
        }
        CodeWorktreeRemovalRecord::Claimed(_) => {
            let stored = load_manifest_sidecar(store, record.authority())?;
            verify_claimed_zero_mutation(&record, &stored, nest_root, deadline)?;
            let removing = store.mark_worktree_removal_removing(&record)?;
            hook.after(FaultBoundary::Removing)?;
            execute_removing(store, removing, &stored, nest_root, deadline, hook)
        }
        CodeWorktreeRemovalRecord::Removing(_) => {
            let stored = load_manifest_sidecar(store, record.authority())?;
            execute_removing(store, record, &stored, nest_root, deadline, hook)
        }
    }
}

fn exact_merge_proof(
    store: &CodeThreadBindingStore,
    lookup: &CodeThreadBindingLookupInput,
    nest_root: &Path,
    deadline: Instant,
) -> Result<CodeMergeProofReceipt, String> {
    match prove_binding_merge_target_before(store, lookup, nest_root, deadline)? {
        Some(CodeMergeProofOutcome::Proven(proof)) => Ok(proof),
        Some(CodeMergeProofOutcome::NotMerged) => {
            Err("SchoolX Code worktree HEAD is not merged into its native target".to_string())
        }
        None => Err("SchoolX Code worktree has no native merge-target authority".to_string()),
    }
}

fn claim_input(
    lookup: &CodeThreadBindingLookupInput,
    proof: CodeMergeProofReceipt,
    stored: &StoredManifest,
) -> CodeWorktreeRemovalClaimInput {
    CodeWorktreeRemovalClaimInput {
        lookup: lookup.clone(),
        merge_proof: proof,
        physical_manifest_digest: stored.digest.clone(),
        git_admin_parent: stored.manifest.git_admin_parent.clone(),
        git_admin_entry: stored.manifest.git_admin_entry.clone(),
    }
}

fn verify_claimed_zero_mutation(
    record: &CodeWorktreeRemovalRecord,
    stored: &StoredManifest,
    nest_root: &Path,
    deadline: Instant,
) -> Result<(), String> {
    let authority = record.authority();
    let layout = pin_layout(&authority.binding, nest_root)?;
    verify_layout_against_authority(&layout, authority, stored)?;
    let original = named_directory_state(
        &layout.root_parent,
        worktree_name(authority)?,
        &stored.manifest.root_identity,
    )?;
    let quarantine = named_directory_state(
        &layout.root_parent,
        OsStr::new(&authority.physical.quarantine_name),
        &stored.manifest.root_identity,
    )?;
    let admin = named_directory_state(
        &layout.admin_parent,
        OsStr::new(&authority.physical.git_admin_entry),
        &stored.manifest.admin_identity,
    )?;
    if (original, quarantine, admin)
        != (
            CoordinateState::Expected,
            CoordinateState::Absent,
            CoordinateState::Expected,
        )
    {
        return Err(
            "SchoolX Code claimed removal no longer has a definitely-not-started physical state"
                .to_string(),
        );
    }
    if read_proof_ref(
        &layout.git_launch,
        &layout.common_dir,
        &layout.common_dir_path,
        authority,
        deadline,
    )?
    .is_some()
    {
        return Err(
            "SchoolX Code claimed removal unexpectedly has a physical proof ref".to_string(),
        );
    }
    let lookup = record.lookup();
    let current_proof = exact_merge_proof_for_record(&lookup, authority, nest_root, deadline)?;
    if current_proof != authority.merge_proof {
        return Err("SchoolX Code removal proof changed after claim".to_string());
    }
    let current = capture_manifest_for_binding(
        &authority.binding,
        &authority.merge_proof,
        nest_root,
        deadline,
    )?;
    if current != *stored {
        return Err("SchoolX Code removal manifest changed after claim".to_string());
    }
    Ok(())
}

fn exact_merge_proof_for_record(
    _lookup: &CodeThreadBindingLookupInput,
    authority: &super::super::CodeWorktreeRemovalAuthority,
    nest_root: &Path,
    deadline: Instant,
) -> Result<CodeMergeProofReceipt, String> {
    let descriptor = crate::code_workspace::worktrees::CodeWorktreeDescriptor {
        execution_mode: authority.binding.execution_mode,
        repository_identity: authority.binding.repository_identity.clone(),
        execution_root: authority.binding.execution_root.clone(),
        base_ref: authority.binding.base_ref.clone(),
        worktree_id: authority.binding.worktree_id.clone(),
    };
    match crate::code_workspace::worktrees::prove_direct_local_ancestry_before(
        &descriptor,
        nest_root,
        &authority.merge_proof.target_ref,
        deadline,
    )? {
        CodeMergeProofOutcome::Proven(proof) => Ok(proof),
        CodeMergeProofOutcome::NotMerged => {
            Err("SchoolX Code removal ancestry proof is no longer valid".to_string())
        }
    }
}

fn execute_removing(
    store: &CodeThreadBindingStore,
    removing: CodeWorktreeRemovalRecord,
    stored: &StoredManifest,
    nest_root: &Path,
    deadline: Instant,
    hook: &mut dyn FaultHook,
) -> Result<CodeWorktreeRemovalRecord, String> {
    let authority = removing.authority();
    let mut boundary = pin_recovery_boundary(authority, stored, nest_root)?;
    verify_recovery_boundary_paths(&boundary, authority, stored)?;
    let states = observe_coordinates(&boundary, authority, stored)?;

    match states {
        (CoordinateState::Expected, CoordinateState::Absent, CoordinateState::Expected) => {
            let lookup = removing.lookup();
            let proof = exact_merge_proof_for_record(&lookup, authority, nest_root, deadline)?;
            if proof != authority.merge_proof {
                return Err("SchoolX Code removing proof changed before first mutation".to_string());
            }
            let current = capture_manifest_for_binding(
                &authority.binding,
                &authority.merge_proof,
                nest_root,
                deadline,
            )?;
            if current != *stored {
                return Err(
                    "SchoolX Code removing manifest changed before first mutation".to_string(),
                );
            }
            verify_recovery_boundary_paths(&boundary, authority, stored)?;
            ensure_proof_ref(
                &boundary.git_launch,
                &boundary.common_dir,
                &boundary.common_dir_path,
                authority,
                deadline,
            )?;
            hook.after(FaultBoundary::ProofRefPinned)?;
            let proof = exact_merge_proof_for_record(&lookup, authority, nest_root, deadline)?;
            if proof != authority.merge_proof {
                return Err("SchoolX Code removal proof drifted after proof-ref pin".to_string());
            }
            let current = capture_manifest_for_binding(
                &authority.binding,
                &authority.merge_proof,
                nest_root,
                deadline,
            )?;
            if current != *stored {
                return Err("SchoolX Code removal manifest drifted after proof-ref pin".to_string());
            }
            verify_recovery_boundary_paths(&boundary, authority, stored)?;
            verify_named_directory(
                &boundary.root_parent,
                worktree_name(authority)?,
                &stored.manifest.root_identity,
            )?;
            quarantine_root(&boundary.root_parent, authority, stored)?;
            hook.after(FaultBoundary::Quarantined)?;
            boundary.root = Some(open_expected_directory_at(
                &boundary.root_parent,
                OsStr::new(&authority.physical.quarantine_name),
                &stored.manifest.root_identity,
                "removal quarantine",
            )?);
        }
        (CoordinateState::Absent, CoordinateState::Expected, CoordinateState::Expected) => {
            require_exact_proof_ref(
                &boundary.git_launch,
                &boundary.common_dir,
                &boundary.common_dir_path,
                authority,
                deadline,
            )?;
            boundary.root = Some(open_expected_directory_at(
                &boundary.root_parent,
                OsStr::new(&authority.physical.quarantine_name),
                &stored.manifest.root_identity,
                "removal quarantine",
            )?);
        }
        (CoordinateState::Absent, CoordinateState::Absent, CoordinateState::Expected)
        | (CoordinateState::Absent, CoordinateState::Absent, CoordinateState::Absent) => {
            require_exact_proof_ref(
                &boundary.git_launch,
                &boundary.common_dir,
                &boundary.common_dir_path,
                authority,
                deadline,
            )?;
        }
        _ => {
            return Err(
                "SchoolX Code removal coordinates contain a replacement or ambiguous state; recovery is sticky"
                    .to_string(),
            );
        }
    }

    let states = observe_coordinates(&boundary, authority, stored)?;
    if states.1 == CoordinateState::Expected {
        let mut path_guard = || verify_recovery_boundary_paths(&boundary, authority, stored);
        delete_manifest_tree(
            &boundary.root,
            &stored.manifest.root_entries,
            &stored.manifest.root_identity,
            hook,
            true,
            &mut path_guard,
        )?;
        verify_recovery_boundary_paths(&boundary, authority, stored)?;
        remove_named_root(
            &boundary.root_parent,
            OsStr::new(&authority.physical.quarantine_name),
            &stored.manifest.root_identity,
        )?;
        hook.after(FaultBoundary::RootDeleted)?;
    }

    verify_recovery_boundary_paths(&boundary, authority, stored)?;
    let states = observe_coordinates(&boundary, authority, stored)?;
    if states.0 != CoordinateState::Absent || states.1 != CoordinateState::Absent {
        return Err(
            "SchoolX Code original or quarantine coordinate was replaced during removal"
                .to_string(),
        );
    }
    if states.2 == CoordinateState::Expected {
        boundary.admin = Some(open_expected_directory_at(
            &boundary.admin_parent,
            OsStr::new(&authority.physical.git_admin_entry),
            &stored.manifest.admin_identity,
            "Git-admin entry",
        )?);
        let mut path_guard = || verify_recovery_boundary_paths(&boundary, authority, stored);
        delete_manifest_tree(
            &boundary.admin,
            &stored.manifest.admin_entries,
            &stored.manifest.admin_identity,
            hook,
            false,
            &mut path_guard,
        )?;
        verify_recovery_boundary_paths(&boundary, authority, stored)?;
        remove_named_root(
            &boundary.admin_parent,
            OsStr::new(&authority.physical.git_admin_entry),
            &stored.manifest.admin_identity,
        )?;
        hook.after(FaultBoundary::AdminDeleted)?;
    } else if states.2 == CoordinateState::Replacement {
        return Err("SchoolX Code Git-admin coordinate was replaced during removal".to_string());
    }

    verify_recovery_boundary_paths(&boundary, authority, stored)?;
    verify_final_absence_and_siblings(&boundary, authority, stored)?;
    hook.after(FaultBoundary::AbsenceVerified)?;
    let capability = VerifiedRemovalAbsence::new(removing.clone());
    let removed = store.finalize_worktree_removal_after_verified_absence(capability)?;
    hook.after(FaultBoundary::Finalized)?;
    cleanup_removed(store, &removed, hook)?;
    Ok(removed)
}

fn cleanup_removed(
    store: &CodeThreadBindingStore,
    removed: &CodeWorktreeRemovalRecord,
    hook: &mut dyn FaultHook,
) -> Result<(), String> {
    let authority = removed.authority();
    let stored = match load_manifest_sidecar(store, authority) {
        Ok(stored) => stored,
        Err(error) if error.starts_with("absent:") => {
            return harden_manifest_absence(store, &authority.physical_manifest_digest)
        }
        Err(error) => return Err(error),
    };
    let common_dir = Path::new(&authority.physical.git_admin_parent)
        .parent()
        .ok_or_else(|| "SchoolX Code removal Git-admin parent has no common dir".to_string())?;
    match fs::symlink_metadata(common_dir) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // The physical removal is finalized, so runtime startup need not
            // be blocked by an offline/moved repository. Keep the sidecar as
            // the durable cleanup-pending marker and retry the exact ref
            // coordinate if the original repository returns.
            return Ok(());
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect removed tombstone common dir: {error}"
            ));
        }
    }
    if repository_identity(common_dir)? != authority.binding.repository_identity {
        return Err("SchoolX Code removed tombstone common-dir identity changed".to_string());
    }
    let common = open_directory_absolute(common_dir, "removal common dir")?;
    if !same_directory_identity(
        &directory_identity(&common)?,
        &stored.manifest.common_dir_identity,
    ) {
        return Err("SchoolX Code removed tombstone common dir was replaced".to_string());
    }
    let git_launch = RemovalGitLaunchAuthority::admit(&common)?;
    delete_proof_ref_if_matches(
        &git_launch,
        &common,
        common_dir,
        authority,
        Instant::now() + GIT_TIMEOUT,
    )?;
    hook.after(FaultBoundary::ProofRefCleaned)?;
    remove_manifest_sidecar(store, &stored)
}

fn capture_manifest(
    store: &CodeThreadBindingStore,
    lookup: &CodeThreadBindingLookupInput,
    proof: &CodeMergeProofReceipt,
    nest_root: &Path,
    deadline: Instant,
) -> Result<StoredManifest, String> {
    let binding = store.lookup(lookup)?.ok_or_else(|| {
        "SchoolX Code removal manifest requires an exact live binding".to_string()
    })?;
    capture_manifest_for_binding(&binding, proof, nest_root, deadline)
}

fn capture_manifest_for_binding(
    binding: &CodeThreadBinding,
    proof: &CodeMergeProofReceipt,
    nest_root: &Path,
    deadline: Instant,
) -> Result<StoredManifest, String> {
    if Instant::now() >= deadline {
        return Err("SchoolX Code removal inspection budget was exhausted".to_string());
    }
    if binding.execution_mode != CodeExecutionMode::Worktree {
        return Err("SchoolX Code physical removal requires a managed worktree".to_string());
    }
    let worktree_id = binding
        .worktree_id
        .as_deref()
        .ok_or_else(|| "SchoolX Code physical removal is missing its worktree id".to_string())?;
    validate_worktree_id(worktree_id)?;
    if proof.repository_identity != binding.repository_identity || proof.worktree_id != worktree_id
    {
        return Err("SchoolX Code physical proof does not match its binding".to_string());
    }

    let layout = pin_layout(binding, nest_root)?;
    verify_repository_storage(&layout, Path::new(&binding.execution_root), deadline)?;
    let tracked = read_tracked_index(
        &layout.git_launch,
        &layout.root,
        Path::new(&binding.execution_root),
        proof,
        deadline,
    )?;
    require_clean_worktree(
        &layout.git_launch,
        &layout.root,
        Path::new(&binding.execution_root),
        deadline,
    )?;
    verify_admin_reciprocal(&layout, binding)?;
    verify_admin_head(&layout, proof)?;

    let root_identity = directory_identity(&layout.root)?;
    let admin_identity = directory_identity(&layout.admin)?;
    let root_entries = scan_managed_root(&layout.root, &tracked)?;
    let admin_entries = scan_admin_tree(&layout.admin)?;
    require_admin_authority_entries(&admin_entries)?;
    let root_parent_siblings =
        snapshot_named_siblings(&layout.root_parent, &[worktree_id.as_bytes()], None)?;
    // The deterministic quarantine name is not known until the journal issues
    // an id. Claim-time snapshots therefore exclude only the live root. A
    // future quarantine child is separately required absent before rename.
    let admin_parent_siblings =
        snapshot_named_siblings(&layout.admin_parent, &[layout.admin_entry.as_bytes()], None)?;

    let manifest = PhysicalRemovalManifest {
        version: MANIFEST_VERSION,
        repository_identity: binding.repository_identity.clone(),
        worktree_id: worktree_id.to_string(),
        managed_root: binding.execution_root.clone(),
        managed_root_parent: Path::new(&binding.execution_root)
            .parent()
            .and_then(Path::to_str)
            .ok_or_else(|| "SchoolX Code managed root has no Unicode parent".to_string())?
            .to_string(),
        git_admin_parent: path_string(&layout.admin_parent_path, "Git-admin parent")?,
        git_admin_entry: layout
            .admin_entry
            .to_str()
            .ok_or_else(|| "SchoolX Code Git-admin entry is not UTF-8".to_string())?
            .to_string(),
        root_parent_identity: directory_identity(&layout.root_parent)?,
        common_dir_identity: directory_identity(&layout.common_dir)?,
        admin_parent_identity: directory_identity(&layout.admin_parent)?,
        root_identity,
        admin_identity,
        root_parent_siblings,
        admin_parent_siblings,
        root_entries,
        admin_entries,
    };
    validate_manifest(&manifest)?;
    let bytes = canonical_manifest_bytes(&manifest)?;
    let digest = sha256_hex(&bytes);

    // Recheck all named handles and reciprocal metadata after the expensive
    // scans so a raced pathname cannot become durable claim authority.
    verify_pinned_layout(&layout, binding)?;
    verify_repository_storage(&layout, Path::new(&binding.execution_root), deadline)?;
    verify_admin_reciprocal(&layout, binding)?;
    verify_admin_head(&layout, proof)?;
    Ok(StoredManifest { digest, manifest })
}

fn pin_layout(binding: &CodeThreadBinding, nest_root: &Path) -> Result<PinnedLayout, String> {
    let worktree_id = binding
        .worktree_id
        .as_deref()
        .ok_or_else(|| "SchoolX Code pinned removal is missing its worktree id".to_string())?;
    let canonical_nest = nest_root
        .canonicalize()
        .map_err(|error| format!("failed to resolve SchoolX nest for removal: {error}"))?;
    if canonical_nest != nest_root {
        return Err("SchoolX Code removal nest is not canonical".to_string());
    }
    let expected_root = canonical_nest
        .join("WORKTREES")
        .join(&binding.repository_identity)
        .join(worktree_id);
    if Path::new(&binding.execution_root) != expected_root {
        return Err("SchoolX Code removal root escaped its managed coordinate".to_string());
    }

    let nest = open_directory_absolute(&canonical_nest, "SchoolX nest")?;
    let worktrees = open_directory_at(&nest, OsStr::new("WORKTREES"), "WORKTREES")?;
    let root_parent = open_directory_at(
        &worktrees,
        OsStr::new(&binding.repository_identity),
        "repository worktree bucket",
    )?;
    let root = open_directory_at(&root_parent, OsStr::new(worktree_id), "managed worktree")?;
    let git_launch = RemovalGitLaunchAuthority::admit(&root)?;

    let git_file =
        read_small_regular_at(&root, OsStr::new(".git"), 64 * 1024, "linked-worktree .git")?;
    let admin_path = parse_gitdir_file(&git_file.bytes, Path::new(&binding.execution_root))?;
    let admin_path = admin_path
        .canonicalize()
        .map_err(|error| format!("failed to resolve linked-worktree Git admin entry: {error}"))?;
    let admin_entry = admin_path
        .file_name()
        .ok_or_else(|| "linked-worktree Git admin entry has no name".to_string())?
        .to_os_string();
    validate_safe_component(&admin_entry, "Git-admin entry")?;
    let admin_parent_path = admin_path
        .parent()
        .ok_or_else(|| "linked-worktree Git admin entry has no parent".to_string())?
        .to_path_buf();
    if admin_parent_path.file_name() != Some(OsStr::new("worktrees")) {
        return Err("linked-worktree Git admin entry is outside common-dir/worktrees".to_string());
    }
    let common_dir_path = admin_parent_path
        .parent()
        .ok_or_else(|| "linked-worktree Git admin parent has no common dir".to_string())?
        .to_path_buf();
    if repository_identity(&common_dir_path)? != binding.repository_identity {
        return Err("SchoolX Code removal common-dir identity changed".to_string());
    }
    let common_dir = open_directory_absolute(&common_dir_path, "Git common directory")?;
    let admin_parent = open_directory_at(&common_dir, OsStr::new("worktrees"), "Git-admin parent")?;
    let admin = open_directory_at(&admin_parent, &admin_entry, "Git-admin entry")?;
    require_same_mount(&nest, &worktrees, "SchoolX WORKTREES")?;
    require_same_mount(&worktrees, &root_parent, "repository worktree bucket")?;
    require_same_mount(&root_parent, &root, "managed worktree root")?;
    require_same_mount(&common_dir, &admin_parent, "Git-admin parent")?;
    require_same_mount(&admin_parent, &admin, "Git-admin entry")?;
    let layout = PinnedLayout {
        nest,
        worktrees,
        root_parent,
        root,
        common_dir,
        admin_parent,
        admin,
        common_dir_path,
        admin_parent_path,
        admin_entry,
        git_launch,
    };
    verify_pinned_layout(&layout, binding)?;
    Ok(layout)
}

fn verify_pinned_layout(layout: &PinnedLayout, binding: &CodeThreadBinding) -> Result<(), String> {
    let worktree_id = binding
        .worktree_id
        .as_deref()
        .ok_or_else(|| "SchoolX Code removal binding lost its worktree id".to_string())?;
    verify_named_directory(
        &layout.nest,
        OsStr::new("WORKTREES"),
        &directory_identity(&layout.worktrees)?,
    )?;
    verify_named_directory(
        &layout.worktrees,
        OsStr::new(&binding.repository_identity),
        &directory_identity(&layout.root_parent)?,
    )?;
    verify_named_directory(
        &layout.root_parent,
        OsStr::new(worktree_id),
        &directory_identity(&layout.root)?,
    )?;
    verify_named_directory(
        &layout.common_dir,
        OsStr::new("worktrees"),
        &directory_identity(&layout.admin_parent)?,
    )?;
    verify_named_directory(
        &layout.admin_parent,
        &layout.admin_entry,
        &directory_identity(&layout.admin)?,
    )
}

struct ReadFile {
    bytes: Vec<u8>,
}

fn verify_admin_reciprocal(
    layout: &PinnedLayout,
    binding: &CodeThreadBinding,
) -> Result<(), String> {
    let root_git = read_small_regular_at(
        &layout.root,
        OsStr::new(".git"),
        64 * 1024,
        "linked-worktree .git",
    )?;
    let pointed_admin = parse_gitdir_file(&root_git.bytes, Path::new(&binding.execution_root))?
        .canonicalize()
        .map_err(|error| format!("failed to resolve reciprocal Git admin entry: {error}"))?;
    let expected_admin = layout.admin_parent_path.join(&layout.admin_entry);
    if pointed_admin != expected_admin {
        return Err("linked-worktree .git does not point to the pinned admin entry".to_string());
    }

    let admin_gitdir = read_small_regular_at(
        &layout.admin,
        OsStr::new("gitdir"),
        64 * 1024,
        "Git-admin gitdir",
    )?;
    let admin_gitdir = parse_plain_path_file(&admin_gitdir.bytes, &expected_admin)?;
    let expected_root_git = Path::new(&binding.execution_root).join(".git");
    let resolved_root_git = admin_gitdir
        .canonicalize()
        .map_err(|error| format!("failed to resolve reciprocal worktree gitfile: {error}"))?;
    let canonical_root_git = expected_root_git
        .canonicalize()
        .map_err(|error| format!("failed to resolve expected worktree gitfile: {error}"))?;
    if resolved_root_git != canonical_root_git {
        return Err("Git-admin gitdir does not point back to the managed root".to_string());
    }

    let commondir = read_small_regular_at(
        &layout.admin,
        OsStr::new("commondir"),
        64 * 1024,
        "Git-admin commondir",
    )?;
    let commondir = parse_plain_path_file(&commondir.bytes, &expected_admin)?;
    let commondir = commondir
        .canonicalize()
        .map_err(|error| format!("failed to resolve Git-admin commondir: {error}"))?;
    if commondir != layout.common_dir_path {
        return Err("Git-admin commondir does not match the pinned common dir".to_string());
    }
    Ok(())
}

fn verify_admin_head(layout: &PinnedLayout, proof: &CodeMergeProofReceipt) -> Result<(), String> {
    let head = read_small_regular_at(
        &layout.admin,
        OsStr::new("HEAD"),
        64 * 1024,
        "Git-admin HEAD",
    )?;
    if one_line(&head.bytes, "Git-admin HEAD")? != proof.head_commit {
        return Err("Git-admin HEAD does not match the persisted removal merge proof".to_string());
    }
    Ok(())
}

fn verify_repository_storage(
    layout: &PinnedLayout,
    root_path: &Path,
    deadline: Instant,
) -> Result<(), String> {
    let captured = run_helper(
        &layout.git_launch,
        &layout.root,
        RemovalGitRequest::RefFormat {
            git_executable: layout.git_launch.executable_string()?,
            expected_target_path: path_string(root_path, "removal Git root")?,
        },
        deadline,
    )?;
    require_success(&captured, "removal ref-format read")?;
    if one_line(&captured.stdout.bytes, "removal ref format")? != "files" {
        return Err("SchoolX Code removal requires the loose-files ref backend".to_string());
    }

    let objects = open_directory_at(
        &layout.common_dir,
        OsStr::new("objects"),
        "Git primary object directory",
    )?;
    require_same_mount(&layout.common_dir, &objects, "Git primary object directory")?;
    if let Some(info) =
        open_optional_directory_at(&objects, OsStr::new("info"), "Git object-info directory")?
    {
        require_same_mount(&objects, &info, "Git object-info directory")?;
        for name in ["alternates", "http-alternates"] {
            let component = CString::new(name)
                .map_err(|_| "Git alternate filename contains NUL".to_string())?;
            match rustix::fs::statat(&info, component.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
                Err(rustix::io::Errno::NOENT) => {}
                Ok(_) => {
                    return Err(
                        "SchoolX Code removal refuses repositories with alternate object storage"
                            .to_string(),
                    )
                }
                Err(error) => {
                    return Err(format!(
                        "failed to inspect Git alternate object storage: {error}"
                    ))
                }
            }
        }
    }
    let mut count = 0_usize;
    verify_owned_object_storage_tree(&objects, 0, &mut count)?;
    Ok(())
}

fn verify_owned_object_storage_tree(
    directory: &fs::File,
    depth: usize,
    count: &mut usize,
) -> Result<(), String> {
    if depth > MAX_OBJECT_STORAGE_DEPTH {
        return Err("SchoolX Code removal Git object storage is nested too deeply".to_string());
    }
    let mut entries = Dir::read_from(directory)
        .map_err(|error| format!("failed to enumerate Git object storage: {error}"))?;
    let mut names = Vec::new();
    while let Some(entry) = entries.read() {
        let entry = entry.map_err(|error| format!("failed to read Git object storage: {error}"))?;
        if !matches!(entry.file_name().to_bytes(), b"." | b"..") {
            names.push(entry.file_name().to_bytes().to_vec());
        }
    }
    names.sort();
    for name in names {
        *count += 1;
        if *count > MAX_OBJECT_STORAGE_ENTRIES {
            return Err("SchoolX Code removal Git object storage is too large".to_string());
        }
        let component =
            CString::new(name).map_err(|_| "Git object-storage name contains NUL".to_string())?;
        let stat = rustix::fs::statat(directory, component.as_c_str(), AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| format!("failed to inspect Git object storage: {error}"))?;
        let file_type = FileType::from_raw_mode(stat.st_mode);
        if file_type.is_dir() {
            let child = open_directory_at_cstr(
                directory,
                component.as_c_str(),
                "Git object-storage directory",
            )?;
            require_same_mount(directory, &child, "Git object-storage directory")?;
            verify_owned_object_storage_tree(&child, depth + 1, count)?;
        } else if file_type.is_file() {
            verify_owned_regular_file_at(
                directory,
                component.as_c_str(),
                "Git object-storage file",
            )?;
        } else {
            return Err("SchoolX Code removal rejects external Git object storage".to_string());
        }
    }
    Ok(())
}

fn verify_owned_regular_file_at(parent: &fs::File, name: &CStr, label: &str) -> Result<(), String> {
    let fd = rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|error| format!("failed to pin {label}: {error}"))?;
    let file = fs::File::from(fd);
    let stat =
        rustix::fs::fstat(&file).map_err(|error| format!("failed to inspect {label}: {error}"))?;
    if !FileType::from_raw_mode(stat.st_mode).is_file() || mount_id(parent)? != mount_id(&file)? {
        return Err(format!("{label} is not owned regular-file storage"));
    }
    Ok(())
}

fn read_tracked_index(
    launch: &RemovalGitLaunchAuthority,
    root: &fs::File,
    root_path: &Path,
    proof: &CodeMergeProofReceipt,
    deadline: Instant,
) -> Result<BTreeMap<Vec<u8>, TrackedEntry>, String> {
    let request = RemovalGitRequest::IndexEntries {
        git_executable: launch.executable_string()?,
        expected_target_path: path_string(root_path, "removal Git root")?,
    };
    let captured = run_helper(launch, root, request, deadline)?;
    require_success(&captured, "removal index read")?;
    if captured.stdout.truncated {
        return Err("SchoolX Code removal index output exceeded its limit".to_string());
    }
    let index = parse_index_entries(&captured.stdout.bytes, proof.head_commit.len())?;
    let head = run_helper(
        launch,
        root,
        RemovalGitRequest::HeadEntries {
            git_executable: launch.executable_string()?,
            expected_target_path: path_string(root_path, "removal Git root")?,
            head_commit: proof.head_commit.clone(),
        },
        deadline,
    )?;
    require_success(&head, "removal HEAD tree read")?;
    let head = parse_head_entries(&head.stdout.bytes, proof.head_commit.len())?;
    if index != head {
        return Err("SchoolX Code removal index does not exactly match HEAD".to_string());
    }
    verify_local_blob_objects(launch, root, root_path, &index, deadline)?;
    Ok(index)
}

fn verify_local_blob_objects(
    launch: &RemovalGitLaunchAuthority,
    root: &fs::File,
    root_path: &Path,
    tracked: &BTreeMap<Vec<u8>, TrackedEntry>,
    deadline: Instant,
) -> Result<(), String> {
    let object_ids = tracked
        .values()
        .map(|entry| entry.object_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    for batch in object_ids.chunks(MAX_OBJECT_TYPE_BATCH) {
        let captured = run_helper(
            launch,
            root,
            RemovalGitRequest::BlobTypes {
                git_executable: launch.executable_string()?,
                expected_target_path: path_string(root_path, "removal Git root")?,
                object_ids: batch.to_vec(),
            },
            deadline,
        )?;
        require_success(&captured, "removal local blob read")?;
        if captured.stdout.truncated {
            return Err("SchoolX Code removal blob-type output exceeded its limit".to_string());
        }
        let mut lines = captured.stdout.bytes.split(|byte| *byte == b'\n');
        for expected in batch {
            let line = lines
                .next()
                .ok_or_else(|| "removal blob-type output ended early".to_string())?;
            let expected_line = format!("{expected} blob");
            if line != expected_line.as_bytes() {
                return Err(
                    "SchoolX Code removal requires every HEAD blob to exist locally".to_string(),
                );
            }
        }
        if lines.any(|line| !line.is_empty()) {
            return Err("removal blob-type output contained extra records".to_string());
        }
    }
    Ok(())
}

fn parse_index_entries(
    bytes: &[u8],
    object_id_length: usize,
) -> Result<BTreeMap<Vec<u8>, TrackedEntry>, String> {
    let mut tracked = BTreeMap::new();
    for record in bytes.split(|byte| *byte == b'\0') {
        if record.is_empty() {
            continue;
        }
        if tracked.len() >= MAX_MANIFEST_ENTRIES {
            return Err(format!(
                "SchoolX Code removal manifest exceeds {MAX_MANIFEST_ENTRIES} tracked entries"
            ));
        }
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| "Git index entry did not contain a path separator".to_string())?;
        let header = std::str::from_utf8(&record[..tab])
            .map_err(|error| format!("Git index entry header was not UTF-8: {error}"))?;
        let mut fields = header.split(' ');
        let mode = fields
            .next()
            .ok_or_else(|| "Git index entry was missing its mode".to_string())?;
        let oid = fields
            .next()
            .ok_or_else(|| "Git index entry was missing its object id".to_string())?;
        let stage = fields
            .next()
            .ok_or_else(|| "Git index entry was missing its stage".to_string())?;
        if fields.next().is_some() || stage != "0" {
            return Err(
                "SchoolX Code removal rejects unmerged or malformed index entries".to_string(),
            );
        }
        if oid.len() != object_id_length
            || !oid
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("SchoolX Code removal index object id is invalid".to_string());
        }
        let kind = parse_tracked_kind(mode)?;
        let path = record[tab + 1..].to_vec();
        validate_relative_bytes(&path)?;
        if path == b".git" || path.starts_with(b".git/") {
            return Err("SchoolX Code removal rejects a tracked .git entry".to_string());
        }
        if tracked
            .insert(
                path,
                TrackedEntry {
                    kind,
                    object_id: oid.to_string(),
                },
            )
            .is_some()
        {
            return Err("SchoolX Code removal index contains a duplicate path".to_string());
        }
    }
    Ok(tracked)
}

fn parse_head_entries(
    bytes: &[u8],
    object_id_length: usize,
) -> Result<BTreeMap<Vec<u8>, TrackedEntry>, String> {
    let mut tracked = BTreeMap::new();
    for record in bytes.split(|byte| *byte == b'\0') {
        if record.is_empty() {
            continue;
        }
        if tracked.len() >= MAX_MANIFEST_ENTRIES {
            return Err(format!(
                "SchoolX Code removal manifest exceeds {MAX_MANIFEST_ENTRIES} HEAD entries"
            ));
        }
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| "Git HEAD entry did not contain a path separator".to_string())?;
        let header = std::str::from_utf8(&record[..tab])
            .map_err(|error| format!("Git HEAD entry header was not UTF-8: {error}"))?;
        let mut fields = header.split(' ');
        let mode = fields
            .next()
            .ok_or_else(|| "Git HEAD entry was missing its mode".to_string())?;
        let object_type = fields
            .next()
            .ok_or_else(|| "Git HEAD entry was missing its object type".to_string())?;
        let oid = fields
            .next()
            .ok_or_else(|| "Git HEAD entry was missing its object id".to_string())?;
        if fields.next().is_some()
            || object_type != "blob"
            || oid.len() != object_id_length
            || !oid
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("Git HEAD entry has invalid object authority".to_string());
        }
        let path = record[tab + 1..].to_vec();
        validate_relative_bytes(&path)?;
        let entry = TrackedEntry {
            kind: parse_tracked_kind(mode)?,
            object_id: oid.to_string(),
        };
        if tracked.insert(path, entry).is_some() {
            return Err("SchoolX Code removal HEAD contains a duplicate path".to_string());
        }
    }
    Ok(tracked)
}

fn parse_tracked_kind(mode: &str) -> Result<TrackedKind, String> {
    match mode {
        "100644" => Ok(TrackedKind::Regular { executable: false }),
        "100755" => Ok(TrackedKind::Regular { executable: true }),
        "120000" => Ok(TrackedKind::Symlink),
        "160000" => Err("SchoolX Code removal refuses submodule/gitlink entries".to_string()),
        _ => Err(format!("SchoolX Code removal rejects tracked mode {mode}")),
    }
}

fn require_clean_worktree(
    launch: &RemovalGitLaunchAuthority,
    root: &fs::File,
    root_path: &Path,
    deadline: Instant,
) -> Result<(), String> {
    let local = run_helper(
        launch,
        root,
        RemovalGitRequest::LocalConfig {
            git_executable: launch.executable_string()?,
            expected_target_path: path_string(root_path, "removal Git root")?,
        },
        deadline,
    )?;
    require_success(&local, "removal local-config read")?;
    let mut overrides = BTreeSet::new();
    let worktree_config =
        crate::code_workspace::collect_local_filter_overrides(&local.stdout.bytes, &mut overrides)?;
    if worktree_config {
        let worktree = run_helper(
            launch,
            root,
            RemovalGitRequest::WorktreeConfigNames {
                git_executable: launch.executable_string()?,
                expected_target_path: path_string(root_path, "removal Git root")?,
            },
            deadline,
        )?;
        require_success(&worktree, "removal worktree-config read")?;
        crate::code_workspace::collect_filter_override_names(
            &worktree.stdout.bytes,
            &mut overrides,
        )?;
    }
    if overrides.len() > MAX_FILTER_KEYS {
        return Err(format!(
            "SchoolX Code removal filter overrides exceed {MAX_FILTER_KEYS} keys"
        ));
    }
    let status = run_helper(
        launch,
        root,
        RemovalGitRequest::Status {
            git_executable: launch.executable_string()?,
            expected_target_path: path_string(root_path, "removal Git root")?,
            disabled_filter_keys: overrides.into_iter().collect(),
        },
        deadline,
    )?;
    require_success(&status, "removal worktree status")?;
    if !status.stdout.bytes.is_empty() {
        return Err("SchoolX Code removal refuses a dirty worktree".to_string());
    }
    Ok(())
}

fn scan_managed_root(
    root: &fs::File,
    tracked: &BTreeMap<Vec<u8>, TrackedEntry>,
) -> Result<Vec<ManifestEntry>, String> {
    let mut allowed_dirs = BTreeSet::new();
    for path in tracked.keys() {
        let components = split_relative_bytes(path)?;
        let mut prefix = Vec::new();
        for component in components.iter().take(components.len().saturating_sub(1)) {
            if !prefix.is_empty() {
                prefix.push(b'/');
            }
            prefix.extend_from_slice(component);
            allowed_dirs.insert(prefix.clone());
        }
    }
    let root_identity = directory_identity(root)?;
    let root_device = root_identity.device;
    let root_mount_id = mount_id(root)?;
    let mut entries = Vec::new();
    let mut seen = BTreeSet::new();
    scan_root_directory(
        root,
        Vec::new(),
        root_device,
        root_mount_id,
        tracked,
        &allowed_dirs,
        &mut seen,
        &mut entries,
    )?;
    let expected = tracked
        .keys()
        .cloned()
        .chain(allowed_dirs.iter().cloned())
        .chain(std::iter::once(b".git".to_vec()))
        .collect::<BTreeSet<_>>();
    if seen != expected {
        return Err("SchoolX Code removal manifest is missing a tracked entry".to_string());
    }
    sort_manifest_entries(&mut entries);
    Ok(entries)
}

#[allow(clippy::too_many_arguments)]
fn scan_root_directory(
    directory: &fs::File,
    prefix: Vec<u8>,
    root_device: u64,
    root_mount_id: u64,
    tracked: &BTreeMap<Vec<u8>, TrackedEntry>,
    allowed_dirs: &BTreeSet<Vec<u8>>,
    seen: &mut BTreeSet<Vec<u8>>,
    entries: &mut Vec<ManifestEntry>,
) -> Result<(), String> {
    if entries.len() >= MAX_MANIFEST_ENTRIES {
        return Err(format!(
            "SchoolX Code removal manifest exceeds {MAX_MANIFEST_ENTRIES} entries"
        ));
    }
    let before = directory_identity(directory)?;
    if before.device != root_device || mount_id(directory)? != root_mount_id {
        return Err(
            "SchoolX Code removal rejects a cross-device or nested-mount directory".to_string(),
        );
    }
    let mut dir = Dir::read_from(directory)
        .map_err(|error| format!("failed to enumerate pinned worktree: {error}"))?;
    let mut names = Vec::new();
    while let Some(entry) = dir.read() {
        let entry = entry.map_err(|error| format!("failed to read worktree entry: {error}"))?;
        let name = entry.file_name().to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        names.push(name.to_vec());
    }
    names.sort();
    for name in names {
        if entries.len() >= MAX_MANIFEST_ENTRIES {
            return Err(format!(
                "SchoolX Code removal manifest exceeds {MAX_MANIFEST_ENTRIES} entries"
            ));
        }
        let component = CString::new(name.clone())
            .map_err(|_| "worktree entry contains an interior NUL".to_string())?;
        let path = join_relative(&prefix, &name);
        let stat = rustix::fs::statat(directory, component.as_c_str(), AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| format!("failed to inspect worktree entry: {error}"))?;
        let file_type = FileType::from_raw_mode(stat.st_mode);
        let kind = if path == b".git" {
            if !file_type.is_file() {
                return Err("linked-worktree .git is not a regular file".to_string());
            }
            ManifestEntryKind::GitFile
        } else if allowed_dirs.contains(&path) {
            if !file_type.is_dir() {
                return Err("tracked ancestor is not a real directory".to_string());
            }
            ManifestEntryKind::Directory
        } else if let Some(expected) = tracked.get(&path) {
            match expected.kind {
                TrackedKind::Regular { .. } if file_type.is_file() => {
                    ManifestEntryKind::RegularFile
                }
                TrackedKind::Symlink if file_type.is_symlink() => ManifestEntryKind::Symlink,
                _ => {
                    return Err(
                        "tracked worktree entry type does not match the Git index".to_string()
                    )
                }
            }
        } else {
            return Err(format!(
                "SchoolX Code removal rejects unexpected worktree entry {}",
                hex::encode(&path)
            ));
        };
        let identity = match kind {
            ManifestEntryKind::Directory => {
                let child =
                    open_directory_at_cstr(directory, component.as_c_str(), "tracked ancestor")?;
                let identity = directory_identity(&child)?;
                if identity.device != stat.st_dev as u64
                    || identity.inode != stat.st_ino
                    || identity.mode != stat.st_mode as u32
                {
                    return Err("tracked ancestor changed while being pinned".to_string());
                }
                identity
            }
            ManifestEntryKind::GitFile | ManifestEntryKind::RegularFile => {
                read_regular_identity_at(directory, component.as_c_str(), &stat)?
            }
            ManifestEntryKind::Symlink => {
                read_symlink_identity_at(directory, component.as_c_str(), &stat)?
            }
            ManifestEntryKind::AdminFile => {
                return Err("root manifest classified an admin-only entry".to_string())
            }
        };
        if identity.device != root_device {
            return Err(
                "SchoolX Code removal rejects a cross-device or nested-mount entry".to_string(),
            );
        }
        if let Some(expected) = tracked.get(&path) {
            if let TrackedKind::Regular { executable } = expected.kind {
                let actual_executable = stat.st_mode & 0o111 != 0;
                if actual_executable != executable {
                    return Err(
                        "tracked worktree executable mode does not match Git HEAD".to_string()
                    );
                }
            }
            let object_id = match expected.kind {
                TrackedKind::Regular { .. } => git_blob_oid_regular_at(
                    directory,
                    component.as_c_str(),
                    &stat,
                    expected.object_id.len(),
                )?,
                TrackedKind::Symlink => git_blob_oid_symlink_at(
                    directory,
                    component.as_c_str(),
                    &stat,
                    expected.object_id.len(),
                )?,
            };
            if object_id != expected.object_id {
                return Err(
                    "tracked worktree content does not match the exact Git object".to_string(),
                );
            }
        }
        if !seen.insert(path.clone()) {
            return Err("SchoolX Code removal manifest contains a duplicate entry".to_string());
        }
        entries.push(ManifestEntry {
            path_hex: hex::encode(&path),
            kind,
            identity: identity.clone(),
        });
        if kind == ManifestEntryKind::Directory {
            let child = open_expected_directory_at_cstr(
                directory,
                component.as_c_str(),
                &identity,
                "tracked ancestor",
            )?;
            scan_root_directory(
                &child,
                path,
                root_device,
                root_mount_id,
                tracked,
                allowed_dirs,
                seen,
                entries,
            )?;
        }
    }
    let after = directory_identity(directory)?;
    if !same_directory_identity(&before, &after) {
        return Err("worktree directory identity changed during manifest capture".to_string());
    }
    Ok(())
}

fn scan_admin_tree(root: &fs::File) -> Result<Vec<ManifestEntry>, String> {
    let root_identity = directory_identity(root)?;
    let root_device = root_identity.device;
    let root_mount_id = mount_id(root)?;
    let mut entries = Vec::new();
    scan_admin_directory(root, Vec::new(), root_device, root_mount_id, &mut entries)?;
    sort_manifest_entries(&mut entries);
    Ok(entries)
}

fn require_admin_authority_entries(entries: &[ManifestEntry]) -> Result<(), String> {
    for required in [b"HEAD".as_slice(), b"commondir", b"gitdir", b"index"] {
        let present = entries.iter().any(|entry| {
            entry.kind == ManifestEntryKind::AdminFile
                && decode_hex_path(&entry.path_hex).is_ok_and(|path| path == required)
        });
        if !present {
            return Err(format!(
                "SchoolX Code Git-admin manifest is missing required file {}",
                String::from_utf8_lossy(required)
            ));
        }
    }
    Ok(())
}

fn scan_admin_directory(
    directory: &fs::File,
    prefix: Vec<u8>,
    root_device: u64,
    root_mount_id: u64,
    entries: &mut Vec<ManifestEntry>,
) -> Result<(), String> {
    if entries.len() >= MAX_MANIFEST_ENTRIES {
        return Err(format!(
            "SchoolX Code Git-admin manifest exceeds {MAX_MANIFEST_ENTRIES} entries"
        ));
    }
    let before = directory_identity(directory)?;
    if before.device != root_device || mount_id(directory)? != root_mount_id {
        return Err(
            "SchoolX Code removal rejects a cross-device or nested-mount Git-admin directory"
                .to_string(),
        );
    }
    let mut dir = Dir::read_from(directory)
        .map_err(|error| format!("failed to enumerate Git-admin entry: {error}"))?;
    let mut names = Vec::new();
    while let Some(entry) = dir.read() {
        let entry = entry.map_err(|error| format!("failed to read Git-admin entry: {error}"))?;
        let name = entry.file_name().to_bytes();
        if name != b"." && name != b".." {
            names.push(name.to_vec());
        }
    }
    names.sort();
    for name in names {
        if entries.len() >= MAX_MANIFEST_ENTRIES {
            return Err(format!(
                "SchoolX Code Git-admin manifest exceeds {MAX_MANIFEST_ENTRIES} entries"
            ));
        }
        if (prefix.is_empty() && name == b"locked") || name.ends_with(b".lock") {
            return Err(
                "SchoolX Code removal refuses a locked or concurrently mutated Git-admin entry"
                    .to_string(),
            );
        }
        let component = CString::new(name.clone())
            .map_err(|_| "Git-admin entry contains an interior NUL".to_string())?;
        let path = join_relative(&prefix, &name);
        let stat = rustix::fs::statat(directory, component.as_c_str(), AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| format!("failed to inspect Git-admin entry: {error}"))?;
        let file_type = FileType::from_raw_mode(stat.st_mode);
        let (kind, identity) = if file_type.is_dir() {
            let child =
                open_directory_at_cstr(directory, component.as_c_str(), "Git-admin directory")?;
            let identity = directory_identity(&child)?;
            if identity.device != stat.st_dev as u64
                || identity.inode != stat.st_ino
                || identity.mode != stat.st_mode as u32
            {
                return Err("Git-admin directory changed while being pinned".to_string());
            }
            (ManifestEntryKind::Directory, identity)
        } else if file_type.is_file() {
            (
                ManifestEntryKind::AdminFile,
                read_regular_identity_at(directory, component.as_c_str(), &stat)?,
            )
        } else {
            return Err(
                "SchoolX Code removal rejects symlink or special Git-admin entries".to_string(),
            );
        };
        if identity.device != root_device {
            return Err(
                "SchoolX Code removal rejects cross-device or nested-mount Git-admin entries"
                    .to_string(),
            );
        }
        entries.push(ManifestEntry {
            path_hex: hex::encode(&path),
            kind,
            identity: identity.clone(),
        });
        if kind == ManifestEntryKind::Directory {
            let child = open_expected_directory_at_cstr(
                directory,
                component.as_c_str(),
                &identity,
                "Git-admin directory",
            )?;
            scan_admin_directory(&child, path, root_device, root_mount_id, entries)?;
        }
    }
    let after = directory_identity(directory)?;
    if !same_directory_identity(&before, &after) {
        return Err("Git-admin directory identity changed during manifest capture".to_string());
    }
    Ok(())
}

fn validate_manifest(manifest: &PhysicalRemovalManifest) -> Result<(), String> {
    if manifest.version != MANIFEST_VERSION {
        return Err(format!(
            "unsupported SchoolX Code removal manifest version {}",
            manifest.version
        ));
    }
    validate_sha256(
        "removal manifest repository identity",
        &manifest.repository_identity,
    )?;
    validate_worktree_id(&manifest.worktree_id)?;
    if manifest.root_entries.len() + manifest.admin_entries.len() > MAX_MANIFEST_ENTRIES {
        return Err(format!(
            "SchoolX Code removal manifest exceeds {MAX_MANIFEST_ENTRIES} entries"
        ));
    }
    let root = Path::new(&manifest.managed_root);
    if !root.is_absolute()
        || root.parent() != Some(Path::new(&manifest.managed_root_parent))
        || root.file_name() != Some(OsStr::new(&manifest.worktree_id))
    {
        return Err("SchoolX Code removal manifest has invalid managed coordinates".to_string());
    }
    let admin_parent = Path::new(&manifest.git_admin_parent);
    if !admin_parent.is_absolute() || admin_parent.file_name() != Some(OsStr::new("worktrees")) {
        return Err("SchoolX Code removal manifest has invalid Git-admin parent".to_string());
    }
    validate_safe_component(OsStr::new(&manifest.git_admin_entry), "Git-admin entry")?;
    validate_node_identity(&manifest.root_parent_identity, true)?;
    validate_node_identity(&manifest.common_dir_identity, true)?;
    validate_node_identity(&manifest.admin_parent_identity, true)?;
    validate_node_identity(&manifest.root_identity, true)?;
    validate_node_identity(&manifest.admin_identity, true)?;
    validate_named_identities(&manifest.root_parent_siblings)?;
    validate_named_identities(&manifest.admin_parent_siblings)?;
    validate_manifest_entries(&manifest.root_entries, true)?;
    validate_manifest_entries(&manifest.admin_entries, false)
}

fn validate_node_identity(identity: &NodeIdentity, directory: bool) -> Result<(), String> {
    if identity.device == 0
        || identity.inode == 0
        || identity.birth_time_seconds <= 0
        || identity.birth_time_nanoseconds >= 1_000_000_000
    {
        return Err("SchoolX Code removal manifest contains an empty node identity".to_string());
    }
    let file_type = FileType::from_raw_mode(identity.mode as _);
    if directory {
        if !file_type.is_dir() || identity.content_sha256.is_some() {
            return Err("SchoolX Code removal directory identity is invalid".to_string());
        }
    } else if let Some(digest) = identity.content_sha256.as_deref() {
        validate_sha256("removal manifest content digest", digest)?;
    }
    Ok(())
}

fn validate_named_identities(values: &[NamedIdentity]) -> Result<(), String> {
    let mut names = BTreeSet::new();
    for value in values {
        let name = decode_hex_path(&value.name_hex)?;
        if name.is_empty() || name.contains(&b'/') || !names.insert(name) {
            return Err("SchoolX Code removal sibling snapshot is invalid".to_string());
        }
        validate_node_identity(&value.identity, false)?;
    }
    Ok(())
}

fn validate_manifest_entries(entries: &[ManifestEntry], root_tree: bool) -> Result<(), String> {
    let mut paths = BTreeSet::new();
    for entry in entries {
        let path = decode_hex_path(&entry.path_hex)?;
        validate_relative_bytes(&path)?;
        if !paths.insert(path.clone()) {
            return Err("SchoolX Code removal manifest contains duplicate paths".to_string());
        }
        let is_directory = entry.kind == ManifestEntryKind::Directory;
        validate_node_identity(&entry.identity, is_directory)?;
        if root_tree {
            if path == b".git" && entry.kind != ManifestEntryKind::GitFile {
                return Err("SchoolX Code removal manifest has invalid .git authority".to_string());
            }
            if path != b".git" && entry.kind == ManifestEntryKind::GitFile {
                return Err("SchoolX Code removal manifest has multiple Git files".to_string());
            }
            if entry.kind == ManifestEntryKind::AdminFile {
                return Err("SchoolX Code root manifest contains an admin-only entry".to_string());
            }
        } else if matches!(
            entry.kind,
            ManifestEntryKind::GitFile
                | ManifestEntryKind::RegularFile
                | ManifestEntryKind::Symlink
        ) {
            return Err("SchoolX Code admin manifest contains an invalid entry kind".to_string());
        }
    }
    if root_tree
        && !entries
            .iter()
            .any(|entry| decode_hex_path(&entry.path_hex).is_ok_and(|path| path == b".git"))
    {
        return Err("SchoolX Code removal manifest is missing linked-worktree .git".to_string());
    }
    Ok(())
}

fn canonical_manifest_bytes(manifest: &PhysicalRemovalManifest) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(manifest)
        .map_err(|error| format!("failed to encode SchoolX Code removal manifest: {error}"))?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(format!(
            "SchoolX Code removal manifest exceeds {MAX_MANIFEST_BYTES} bytes"
        ));
    }
    Ok(bytes)
}

fn persist_manifest_sidecar(
    store: &CodeThreadBindingStore,
    stored: &StoredManifest,
) -> Result<(), String> {
    validate_sha256("removal manifest digest", &stored.digest)?;
    validate_manifest(&stored.manifest)?;
    let bytes = canonical_manifest_bytes(&stored.manifest)?;
    if sha256_hex(&bytes) != stored.digest {
        return Err("SchoolX Code removal manifest digest changed before persistence".to_string());
    }
    let directory = ensure_manifest_directory(store)?;
    let final_name = format!("{}.json", stored.digest);
    match read_manifest_file_at(&directory.handle, &final_name) {
        Ok(existing) => {
            if existing.bytes == bytes {
                rustix::fs::fsync(&directory.handle).map_err(|error| {
                    format!("failed to sync existing removal manifest directory: {error}")
                })?;
                verify_manifest_directory_named(store, &directory.handle)?;
                return Ok(());
            }
            return Err("SchoolX Code removal manifest sidecar was replaced".to_string());
        }
        Err(error) if error.starts_with("absent:") => {}
        Err(error) => return Err(error),
    }

    let temporary_name = format!(
        ".{}.{}.tmp",
        stored.digest,
        uuid::Uuid::new_v4().hyphenated()
    );
    let write_result = (|| {
        let temporary = CString::new(temporary_name.as_bytes())
            .map_err(|_| "removal manifest temp name contains NUL".to_string())?;
        let fd = rustix::fs::openat(
            &directory.handle,
            temporary.as_c_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|error| format!("failed to create removal manifest sidecar temp: {error}"))?;
        let mut file = fs::File::from(fd);
        file.write_all(&bytes)
            .map_err(|error| format!("failed to write removal manifest sidecar: {error}"))?;
        file.sync_all()
            .map_err(|error| format!("failed to sync removal manifest sidecar: {error}"))
    })();
    if let Err(error) = write_result {
        let _ = unlink_manifest_temp(&directory.handle, &temporary_name);
        return Err(error);
    }
    let temporary = CString::new(temporary_name.as_bytes())
        .map_err(|_| "removal manifest temp name contains NUL".to_string())?;
    let final_component = CString::new(final_name.as_bytes())
        .map_err(|_| "removal manifest filename contains NUL".to_string())?;
    match rustix::fs::renameat_with(
        &directory.handle,
        temporary.as_c_str(),
        &directory.handle,
        final_component.as_c_str(),
        RenameFlags::NOREPLACE,
    ) {
        Ok(()) => {}
        Err(rustix::io::Errno::EXIST) => {
            unlink_manifest_temp(&directory.handle, &temporary_name)?;
            let existing = read_manifest_file_at(&directory.handle, &final_name)?;
            if existing.bytes == bytes {
                rustix::fs::fsync(&directory.handle).map_err(|sync_error| {
                    format!("failed to sync raced removal manifest directory: {sync_error}")
                })?;
                verify_manifest_directory_named(store, &directory.handle)?;
                return Ok(());
            }
            return Err("SchoolX Code removal manifest sidecar was replaced".to_string());
        }
        Err(error) => {
            let cleanup = unlink_manifest_temp(&directory.handle, &temporary_name).err();
            return Err(match cleanup {
                Some(cleanup) => format!(
                    "failed to publish removal manifest sidecar: {error}; temp cleanup also failed: {cleanup}"
                ),
                None => format!("failed to publish removal manifest sidecar: {error}"),
            });
        }
    }
    rustix::fs::fsync(&directory.handle)
        .map_err(|error| format!("failed to sync removal manifest directory: {error}"))?;
    let persisted = read_manifest_file_at(&directory.handle, &final_name)?;
    if persisted.bytes != bytes {
        return Err("SchoolX Code removal manifest sidecar changed after commit".to_string());
    }
    verify_manifest_directory_named(store, &directory.handle)?;
    Ok(())
}

fn unlink_manifest_temp(directory: &fs::File, name: &str) -> Result<(), String> {
    let component = CString::new(name.as_bytes())
        .map_err(|_| "removal manifest temp name contains NUL".to_string())?;
    match rustix::fs::unlinkat(directory, component.as_c_str(), AtFlags::empty()) {
        Ok(()) | Err(rustix::io::Errno::NOENT) => Ok(()),
        Err(error) => Err(format!("failed to remove removal manifest temp: {error}")),
    }
}

fn remove_manifest_sidecar(
    store: &CodeThreadBindingStore,
    stored: &StoredManifest,
) -> Result<(), String> {
    let Some(directory) = open_manifest_directory(store)? else {
        return harden_manifest_absence(store, &stored.digest);
    };
    let filename = format!("{}.json", stored.digest);
    let current = match read_manifest_file_at(&directory.handle, &filename) {
        Ok(current) => current,
        Err(error) if error.starts_with("absent:") => {
            rustix::fs::fsync(&directory.handle).map_err(|sync_error| {
                format!("failed to sync absent removal manifest sidecar: {sync_error}")
            })?;
            return verify_manifest_directory_named(store, &directory.handle);
        }
        Err(error) => return Err(error),
    };
    if current.bytes != canonical_manifest_bytes(&stored.manifest)?
        || sha256_hex(&current.bytes) != stored.digest
    {
        return Err("SchoolX Code removal manifest sidecar changed before cleanup".to_string());
    }
    let latest = read_manifest_file_at(&directory.handle, &filename)?;
    if latest != current {
        return Err(
            "SchoolX Code removal manifest sidecar was replaced before cleanup".to_string(),
        );
    }
    let component = CString::new(filename.as_bytes())
        .map_err(|_| "removal manifest filename contains NUL".to_string())?;
    verify_manifest_file_named(&directory.handle, component.as_c_str(), &latest.identity)?;
    rustix::fs::unlinkat(&directory.handle, component.as_c_str(), AtFlags::empty())
        .map_err(|error| format!("failed to remove exact removal manifest sidecar: {error}"))?;
    rustix::fs::fsync(&directory.handle)
        .map_err(|error| format!("failed to sync removal manifest cleanup: {error}"))?;
    verify_manifest_directory_named(store, &directory.handle)?;
    match read_manifest_file_at(&directory.handle, &filename) {
        Err(error) if error.starts_with("absent:") => Ok(()),
        Ok(_) => Err("SchoolX Code removal manifest sidecar remained after cleanup".to_string()),
        Err(error) => Err(error),
    }
}

fn load_manifest_sidecar(
    store: &CodeThreadBindingStore,
    authority: &super::super::CodeWorktreeRemovalAuthority,
) -> Result<StoredManifest, String> {
    let digest = &authority.physical_manifest_digest;
    validate_sha256("removal manifest digest", digest)?;
    let directory = open_manifest_directory(store)?
        .ok_or_else(|| format!("absent:removal manifest directory for {digest}"))?;
    let bytes = read_manifest_file_at(&directory.handle, &format!("{digest}.json"))?.bytes;
    if bytes.len() > MAX_MANIFEST_BYTES || sha256_hex(&bytes) != *digest {
        return Err("SchoolX Code removal manifest sidecar digest is invalid".to_string());
    }
    let manifest: PhysicalRemovalManifest = serde_json::from_slice(&bytes)
        .map_err(|error| format!("SchoolX Code removal manifest sidecar is invalid: {error}"))?;
    validate_manifest(&manifest)?;
    if canonical_manifest_bytes(&manifest)? != bytes {
        return Err("SchoolX Code removal manifest sidecar is not canonical".to_string());
    }
    if manifest.repository_identity != authority.binding.repository_identity
        || manifest.worktree_id != authority.merge_proof.worktree_id
        || manifest.managed_root != authority.physical.managed_root
        || manifest.managed_root_parent != authority.physical.managed_root_parent
        || manifest.git_admin_parent != authority.physical.git_admin_parent
        || manifest.git_admin_entry != authority.physical.git_admin_entry
    {
        return Err("SchoolX Code removal manifest does not match journal authority".to_string());
    }
    verify_manifest_directory_named(store, &directory.handle)?;
    Ok(StoredManifest {
        digest: digest.clone(),
        manifest,
    })
}

fn open_manifest_directory(
    store: &CodeThreadBindingStore,
) -> Result<Option<ManifestDirectory>, String> {
    let parent = open_directory_absolute(&store.code_dir, "SchoolX Code data directory")?;
    let Some(handle) = open_optional_directory_at(
        &parent,
        OsStr::new(MANIFEST_DIRECTORY),
        "removal manifest directory",
    )?
    else {
        return Ok(None);
    };
    require_same_mount(&parent, &handle, "removal manifest directory")?;
    Ok(Some(ManifestDirectory { handle }))
}

fn verify_manifest_directory_named(
    store: &CodeThreadBindingStore,
    expected: &fs::File,
) -> Result<(), String> {
    let current = open_manifest_directory(store)?
        .ok_or_else(|| "SchoolX Code removal manifest directory disappeared".to_string())?;
    if !same_directory_identity(
        &directory_identity(&current.handle)?,
        &directory_identity(expected)?,
    ) {
        return Err("SchoolX Code removal manifest directory was replaced".to_string());
    }
    Ok(())
}

fn harden_manifest_absence(store: &CodeThreadBindingStore, digest: &str) -> Result<(), String> {
    let parent = open_directory_absolute(&store.code_dir, "SchoolX Code data directory")?;
    let Some(directory) = open_manifest_directory(store)? else {
        return rustix::fs::fsync(&parent)
            .map_err(|error| format!("failed to sync absent removal manifest directory: {error}"));
    };
    let filename = CString::new(format!("{digest}.json"))
        .map_err(|_| "removal manifest filename contains NUL".to_string())?;
    match rustix::fs::statat(
        &directory.handle,
        filename.as_c_str(),
        AtFlags::SYMLINK_NOFOLLOW,
    ) {
        Err(rustix::io::Errno::NOENT) => {}
        Ok(_) => return Err("SchoolX Code removal manifest sidecar still exists".to_string()),
        Err(error) => {
            return Err(format!(
                "failed to verify removal manifest absence: {error}"
            ))
        }
    }
    rustix::fs::fsync(&directory.handle)
        .map_err(|error| format!("failed to sync removal manifest absence: {error}"))?;
    verify_manifest_directory_named(store, &directory.handle)
}

fn ensure_manifest_directory(store: &CodeThreadBindingStore) -> Result<ManifestDirectory, String> {
    if let Some(directory) = open_manifest_directory(store)? {
        directory
            .handle
            .set_permissions(fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("failed to secure removal manifest directory: {error}"))?;
        return Ok(directory);
    }
    let parent = open_directory_absolute(&store.code_dir, "SchoolX Code data directory")?;
    let component = CString::new(MANIFEST_DIRECTORY)
        .map_err(|_| "removal manifest directory contains NUL".to_string())?;
    match rustix::fs::mkdirat(&parent, component.as_c_str(), Mode::RWXU) {
        Ok(()) | Err(rustix::io::Errno::EXIST) => {}
        Err(error) => {
            return Err(format!(
                "failed to create removal manifest directory: {error}"
            ))
        }
    }
    rustix::fs::fsync(&parent)
        .map_err(|error| format!("failed to sync SchoolX Code data directory: {error}"))?;
    let handle =
        open_directory_at_cstr(&parent, component.as_c_str(), "removal manifest directory")?;
    require_same_mount(&parent, &handle, "removal manifest directory")?;
    handle
        .set_permissions(fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("failed to secure removal manifest directory: {error}"))?;
    rustix::fs::fsync(&handle)
        .map_err(|error| format!("failed to sync removal manifest directory: {error}"))?;
    Ok(ManifestDirectory { handle })
}

fn read_manifest_file_at(directory: &fs::File, name: &str) -> Result<ManifestFileRead, String> {
    let component = CString::new(name.as_bytes())
        .map_err(|_| "removal manifest filename contains NUL".to_string())?;
    let fd = match rustix::fs::openat(
        directory,
        component.as_c_str(),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Err(format!("absent:{name}")),
        Err(error) => return Err(format!("failed to open removal manifest sidecar: {error}")),
    };
    let mut file = fs::File::from(fd);
    if mount_id(directory)? != mount_id(&file)? {
        return Err("SchoolX Code removal manifest sidecar crosses a mount boundary".to_string());
    }
    let before = rustix::fs::fstat(&file)
        .map_err(|error| format!("failed to inspect removal manifest sidecar: {error}"))?;
    if !FileType::from_raw_mode(before.st_mode).is_file()
        || before.st_size < 0
        || before.st_size as usize > MAX_MANIFEST_BYTES
    {
        return Err("SchoolX Code removal manifest sidecar is not a bounded file".to_string());
    }
    let mut bytes = Vec::with_capacity(before.st_size as usize);
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read removal manifest sidecar: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("failed to sync removal manifest sidecar: {error}"))?;
    let after = rustix::fs::fstat(&file)
        .map_err(|error| format!("failed to re-inspect removal manifest sidecar: {error}"))?;
    if before.st_dev != after.st_dev
        || before.st_ino != after.st_ino
        || before.st_mode != after.st_mode
        || before.st_size != after.st_size
        || bytes.len() != before.st_size as usize
    {
        return Err("SchoolX Code removal manifest sidecar changed while reading".to_string());
    }
    let identity = node_identity_from_fd(&file, &before, Some(sha256_hex(&bytes)))?;
    verify_manifest_file_named(directory, component.as_c_str(), &identity)?;
    Ok(ManifestFileRead { bytes, identity })
}

fn verify_manifest_file_named(
    directory: &fs::File,
    name: &CStr,
    expected: &NodeIdentity,
) -> Result<(), String> {
    let stat = rustix::fs::statat(directory, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| format!("failed to re-inspect removal manifest sidecar: {error}"))?;
    if !FileType::from_raw_mode(stat.st_mode).is_file() {
        return Err("SchoolX Code removal manifest sidecar was replaced".to_string());
    }
    let fd = rustix::fs::openat(
        directory,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|error| format!("failed to re-pin removal manifest sidecar: {error}"))?;
    let file = fs::File::from(fd);
    if mount_id(directory)? != mount_id(&file)? {
        return Err("SchoolX Code removal manifest sidecar crosses a mount boundary".to_string());
    }
    let actual = rustix::fs::fstat(&file)
        .map_err(|error| format!("failed to re-inspect removal manifest sidecar: {error}"))?;
    let identity = node_identity_from_fd(&file, &actual, expected.content_sha256.clone())?;
    if !same_named_identity(&identity, expected) {
        return Err("SchoolX Code removal manifest sidecar was replaced".to_string());
    }
    Ok(())
}

fn snapshot_named_siblings(
    parent: &fs::File,
    excluded_names: &[&[u8]],
    excluded_prefix: Option<&[u8]>,
) -> Result<Vec<NamedIdentity>, String> {
    let mut dir = Dir::read_from(parent)
        .map_err(|error| format!("failed to enumerate removal parent: {error}"))?;
    let mut names = Vec::new();
    while let Some(entry) = dir.read() {
        let entry = entry.map_err(|error| format!("failed to read removal parent: {error}"))?;
        let name = entry.file_name().to_bytes();
        if name == b"."
            || name == b".."
            || excluded_names.contains(&name)
            || excluded_prefix.is_some_and(|prefix| name.starts_with(prefix))
        {
            continue;
        }
        names.push(name.to_vec());
    }
    names.sort();
    let mut snapshot = Vec::with_capacity(names.len());
    for name in names {
        let component = CString::new(name.clone())
            .map_err(|_| "removal sibling name contains NUL".to_string())?;
        let stat = rustix::fs::statat(parent, component.as_c_str(), AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|error| format!("failed to inspect removal sibling: {error}"))?;
        snapshot.push(NamedIdentity {
            name_hex: hex::encode(name),
            identity: node_identity_from_at(parent, component.as_c_str(), &stat, None)?,
        });
    }
    Ok(snapshot)
}

fn verify_sibling_snapshot(
    parent: &fs::File,
    expected: &[NamedIdentity],
    excluded_names: &[&[u8]],
    excluded_prefix: Option<&[u8]>,
) -> Result<(), String> {
    let current = snapshot_named_siblings(parent, excluded_names, excluded_prefix)?;
    if current != expected {
        return Err("SchoolX Code removal sibling set changed during deletion".to_string());
    }
    Ok(())
}

fn node_identity_from_parts(
    stat: &rustix::fs::Stat,
    incarnation: (i64, u32, u64),
    digest: Option<String>,
) -> Result<NodeIdentity, String> {
    if stat.st_size < 0 || incarnation.0 <= 0 || incarnation.1 >= 1_000_000_000 {
        return Err("SchoolX Code removal requires stable birth-time identity".to_string());
    }
    Ok(NodeIdentity {
        device: stat.st_dev as u64,
        inode: stat.st_ino,
        mode: stat.st_mode as u32,
        size: stat.st_size as u64,
        birth_time_seconds: incarnation.0,
        birth_time_nanoseconds: incarnation.1,
        generation: incarnation.2,
        content_sha256: digest,
    })
}

fn node_identity_from_fd(
    file: &fs::File,
    stat: &rustix::fs::Stat,
    digest: Option<String>,
) -> Result<NodeIdentity, String> {
    node_identity_from_parts(stat, incarnation_from_fd(file, stat)?, digest)
}

fn node_identity_from_at(
    parent: &fs::File,
    name: &CStr,
    stat: &rustix::fs::Stat,
    digest: Option<String>,
) -> Result<NodeIdentity, String> {
    node_identity_from_parts(stat, incarnation_from_at(parent, name, stat)?, digest)
}

#[cfg(target_os = "linux")]
fn incarnation_from_fd(
    file: &fs::File,
    expected: &rustix::fs::Stat,
) -> Result<(i64, u32, u64), String> {
    use rustix::fs::StatxFlags;

    let stat = rustix::fs::statx(
        file,
        Path::new(""),
        AtFlags::EMPTY_PATH | AtFlags::NO_AUTOMOUNT,
        StatxFlags::BASIC_STATS | StatxFlags::BTIME,
    )
    .map_err(|error| format!("failed to inspect removal birth identity: {error}"))?;
    validate_linux_incarnation(&stat, expected)
}

#[cfg(target_os = "linux")]
fn incarnation_from_at(
    parent: &fs::File,
    name: &CStr,
    expected: &rustix::fs::Stat,
) -> Result<(i64, u32, u64), String> {
    use rustix::fs::StatxFlags;

    let stat = rustix::fs::statx(
        parent,
        name,
        AtFlags::SYMLINK_NOFOLLOW | AtFlags::NO_AUTOMOUNT,
        StatxFlags::BASIC_STATS | StatxFlags::BTIME,
    )
    .map_err(|error| format!("failed to inspect removal birth identity: {error}"))?;
    validate_linux_incarnation(&stat, expected)
}

#[cfg(target_os = "linux")]
fn validate_linux_incarnation(
    stat: &rustix::fs::Statx,
    expected: &rustix::fs::Stat,
) -> Result<(i64, u32, u64), String> {
    use rustix::fs::StatxFlags;

    if stat.stx_mask & StatxFlags::BTIME.bits() == 0
        || stat.stx_ino != expected.st_ino
        || stat.stx_mode as u32 != expected.st_mode as u32
        || rustix::fs::makedev(stat.stx_dev_major, stat.stx_dev_minor) as u64
            != expected.st_dev as u64
    {
        return Err("SchoolX Code removal birth identity is unavailable or raced".to_string());
    }
    Ok((stat.stx_btime.tv_sec, stat.stx_btime.tv_nsec, 0))
}

#[cfg(target_os = "macos")]
fn incarnation_from_fd(
    _file: &fs::File,
    stat: &rustix::fs::Stat,
) -> Result<(i64, u32, u64), String> {
    macos_incarnation(stat)
}

#[cfg(target_os = "macos")]
fn incarnation_from_at(
    _parent: &fs::File,
    _name: &CStr,
    stat: &rustix::fs::Stat,
) -> Result<(i64, u32, u64), String> {
    macos_incarnation(stat)
}

#[cfg(target_os = "macos")]
fn macos_incarnation(stat: &rustix::fs::Stat) -> Result<(i64, u32, u64), String> {
    let nanoseconds = u32::try_from(stat.st_birthtime_nsec)
        .map_err(|_| "SchoolX Code removal birth-time nanoseconds are invalid".to_string())?;
    Ok((stat.st_birthtime, nanoseconds, stat.st_gen as u64))
}

fn directory_identity(directory: &fs::File) -> Result<NodeIdentity, String> {
    let stat = rustix::fs::fstat(directory)
        .map_err(|error| format!("failed to inspect pinned removal directory: {error}"))?;
    if !FileType::from_raw_mode(stat.st_mode).is_dir() {
        return Err("pinned SchoolX Code removal handle is not a directory".to_string());
    }
    node_identity_from_fd(directory, &stat, None)
}

#[cfg(target_os = "linux")]
fn mount_id(file: &fs::File) -> Result<u64, String> {
    use rustix::fs::StatxFlags;

    let stat = rustix::fs::statx(
        file,
        Path::new(""),
        AtFlags::EMPTY_PATH | AtFlags::NO_AUTOMOUNT,
        StatxFlags::MNT_ID,
    )
    .map_err(|error| format!("failed to inspect removal mount identity: {error}"))?;
    if stat.stx_mask & StatxFlags::MNT_ID.bits() == 0 || stat.stx_mnt_id == 0 {
        return Err("SchoolX Code removal requires Linux mount-id authority".to_string());
    }
    Ok(stat.stx_mnt_id)
}

#[cfg(target_os = "macos")]
fn mount_id(file: &fs::File) -> Result<u64, String> {
    let stat = rustix::fs::fstatvfs(file)
        .map_err(|error| format!("failed to inspect removal mount identity: {error}"))?;
    if stat.f_fsid == 0 {
        return Err("SchoolX Code removal requires macOS mount identity".to_string());
    }
    Ok(stat.f_fsid)
}

fn same_directory_identity(left: &NodeIdentity, right: &NodeIdentity) -> bool {
    left.device == right.device
        && left.inode == right.inode
        && left.mode == right.mode
        && left.birth_time_seconds == right.birth_time_seconds
        && left.birth_time_nanoseconds == right.birth_time_nanoseconds
        && left.generation == right.generation
        && FileType::from_raw_mode(left.mode as _).is_dir()
        && FileType::from_raw_mode(right.mode as _).is_dir()
}

fn same_named_identity(left: &NodeIdentity, right: &NodeIdentity) -> bool {
    left.device == right.device
        && left.inode == right.inode
        && left.mode == right.mode
        && left.size == right.size
        && left.birth_time_seconds == right.birth_time_seconds
        && left.birth_time_nanoseconds == right.birth_time_nanoseconds
        && left.generation == right.generation
        && left.content_sha256 == right.content_sha256
}

fn require_same_mount(parent: &fs::File, child: &fs::File, label: &str) -> Result<(), String> {
    let parent_identity = directory_identity(parent)?;
    let child_identity = directory_identity(child)?;
    if parent_identity.device != child_identity.device || mount_id(parent)? != mount_id(child)? {
        return Err(format!("{label} crosses a nested mount boundary"));
    }
    Ok(())
}

fn read_regular_identity_at(
    parent: &fs::File,
    name: &CStr,
    expected_stat: &rustix::fs::Stat,
) -> Result<NodeIdentity, String> {
    let fd = rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|error| format!("failed to pin manifest file: {error}"))?;
    let before = rustix::fs::fstat(&fd)
        .map_err(|error| format!("failed to inspect manifest file: {error}"))?;
    if !FileType::from_raw_mode(before.st_mode).is_file()
        || before.st_dev != expected_stat.st_dev
        || before.st_ino != expected_stat.st_ino
        || before.st_mode != expected_stat.st_mode
        || before.st_size != expected_stat.st_size
        || before.st_size < 0
        || before.st_size as u64 > MAX_FILE_BYTES
    {
        return Err("manifest file identity changed before hashing".to_string());
    }
    let mut file = fs::File::from(fd);
    if mount_id(parent)? != mount_id(&file)? {
        return Err("SchoolX Code removal rejects a nested-mount manifest file".to_string());
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to hash manifest file: {error}"))?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > MAX_FILE_BYTES {
            return Err("manifest file exceeded the removal size limit".to_string());
        }
        hasher.update(&buffer[..read]);
    }
    let after = rustix::fs::fstat(&file)
        .map_err(|error| format!("failed to re-inspect manifest file: {error}"))?;
    if before.st_dev != after.st_dev
        || before.st_ino != after.st_ino
        || before.st_mode != after.st_mode
        || before.st_size != after.st_size
        || total != before.st_size as u64
    {
        return Err("manifest file changed while hashing".to_string());
    }
    node_identity_from_fd(&file, &before, Some(hex::encode(hasher.finalize())))
}

fn read_symlink_identity_at(
    parent: &fs::File,
    name: &CStr,
    expected_stat: &rustix::fs::Stat,
) -> Result<NodeIdentity, String> {
    if !FileType::from_raw_mode(expected_stat.st_mode).is_symlink() {
        return Err("manifest symlink changed type".to_string());
    }
    let first = rustix::fs::readlinkat(parent, name, Vec::new())
        .map_err(|error| format!("failed to read manifest symlink: {error}"))?;
    let after = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| format!("failed to re-inspect manifest symlink: {error}"))?;
    let second = rustix::fs::readlinkat(parent, name, Vec::new())
        .map_err(|error| format!("failed to re-read manifest symlink: {error}"))?;
    if expected_stat.st_dev != after.st_dev
        || expected_stat.st_ino != after.st_ino
        || expected_stat.st_mode != after.st_mode
        || expected_stat.st_size != after.st_size
        || first.as_bytes() != second.as_bytes()
    {
        return Err("manifest symlink changed during capture".to_string());
    }
    node_identity_from_at(
        parent,
        name,
        expected_stat,
        Some(sha256_hex(first.as_bytes())),
    )
}

fn git_blob_oid_regular_at(
    parent: &fs::File,
    name: &CStr,
    expected_stat: &rustix::fs::Stat,
    object_id_length: usize,
) -> Result<String, String> {
    match object_id_length {
        40 => hash_regular_git_blob::<sha1::Sha1>(parent, name, expected_stat),
        64 => hash_regular_git_blob::<Sha256>(parent, name, expected_stat),
        _ => Err("SchoolX Code removal Git object format is unsupported".to_string()),
    }
}

fn hash_regular_git_blob<D>(
    parent: &fs::File,
    name: &CStr,
    expected_stat: &rustix::fs::Stat,
) -> Result<String, String>
where
    D: Digest + Default,
{
    let fd = rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|error| format!("failed to pin tracked Git blob: {error}"))?;
    let before = rustix::fs::fstat(&fd)
        .map_err(|error| format!("failed to inspect tracked Git blob: {error}"))?;
    if !FileType::from_raw_mode(before.st_mode).is_file()
        || before.st_dev != expected_stat.st_dev
        || before.st_ino != expected_stat.st_ino
        || before.st_mode != expected_stat.st_mode
        || before.st_size != expected_stat.st_size
        || before.st_size < 0
        || before.st_size as u64 > MAX_FILE_BYTES
    {
        return Err("tracked Git blob identity changed before hashing".to_string());
    }
    let mut file = fs::File::from(fd);
    let mut hasher = D::default();
    hasher.update(format!("blob {}\0", before.st_size).as_bytes());
    let mut buffer = [0_u8; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("failed to hash tracked Git blob: {error}"))?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > MAX_FILE_BYTES {
            return Err("tracked Git blob exceeded the removal size limit".to_string());
        }
        hasher.update(&buffer[..read]);
    }
    let after = rustix::fs::fstat(&file)
        .map_err(|error| format!("failed to re-inspect tracked Git blob: {error}"))?;
    if before.st_dev != after.st_dev
        || before.st_ino != after.st_ino
        || before.st_mode != after.st_mode
        || before.st_size != after.st_size
        || total != before.st_size as u64
    {
        return Err("tracked Git blob changed while hashing".to_string());
    }
    Ok(hex::encode(hasher.finalize()))
}

fn git_blob_oid_symlink_at(
    parent: &fs::File,
    name: &CStr,
    expected_stat: &rustix::fs::Stat,
    object_id_length: usize,
) -> Result<String, String> {
    if !FileType::from_raw_mode(expected_stat.st_mode).is_symlink() {
        return Err("tracked Git symlink changed type".to_string());
    }
    let first = rustix::fs::readlinkat(parent, name, Vec::new())
        .map_err(|error| format!("failed to read tracked Git symlink: {error}"))?;
    let after = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| format!("failed to re-inspect tracked Git symlink: {error}"))?;
    let second = rustix::fs::readlinkat(parent, name, Vec::new())
        .map_err(|error| format!("failed to re-read tracked Git symlink: {error}"))?;
    if expected_stat.st_dev != after.st_dev
        || expected_stat.st_ino != after.st_ino
        || expected_stat.st_mode != after.st_mode
        || expected_stat.st_size != after.st_size
        || first.as_bytes() != second.as_bytes()
    {
        return Err("tracked Git symlink changed while hashing".to_string());
    }
    match object_id_length {
        40 => Ok(hash_git_blob_bytes::<sha1::Sha1>(first.as_bytes())),
        64 => Ok(hash_git_blob_bytes::<Sha256>(first.as_bytes())),
        _ => Err("SchoolX Code removal Git object format is unsupported".to_string()),
    }
}

fn hash_git_blob_bytes<D>(bytes: &[u8]) -> String
where
    D: Digest + Default,
{
    let mut hasher = D::default();
    hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn read_small_regular_at(
    parent: &fs::File,
    name: &OsStr,
    limit: usize,
    label: &str,
) -> Result<ReadFile, String> {
    let component = CString::new(name.as_bytes())
        .map_err(|_| format!("{label} name contains an interior NUL"))?;
    let stat = rustix::fs::statat(parent, component.as_c_str(), AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| format!("failed to inspect {label}: {error}"))?;
    if stat.st_size < 0 || stat.st_size as usize > limit {
        return Err(format!("{label} exceeds its size limit"));
    }
    let identity = read_regular_identity_at(parent, component.as_c_str(), &stat)?;
    let fd = rustix::fs::openat(
        parent,
        component.as_c_str(),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|error| format!("failed to open {label}: {error}"))?;
    let file = fs::File::from(fd);
    let mut bytes = Vec::with_capacity(stat.st_size as usize);
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read {label}: {error}"))?;
    if bytes.len() > limit || sha256_hex(&bytes) != identity.content_sha256.as_deref().unwrap_or("")
    {
        return Err(format!("{label} changed while reading"));
    }
    Ok(ReadFile { bytes })
}

fn open_directory_absolute(path: &Path, label: &str) -> Result<fs::File, String> {
    if !path.is_absolute() {
        return Err(format!("{label} must be an absolute directory"));
    }
    let mut directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(Path::new("/"))
        .map_err(|error| format!("failed to pin filesystem root for {label}: {error}"))?;
    for component in path.components() {
        match component {
            std::path::Component::RootDir => {}
            std::path::Component::Normal(name) => {
                directory = open_directory_at(&directory, name, label)?;
            }
            _ => return Err(format!("{label} contains a non-canonical path component")),
        }
    }
    directory_identity(&directory)?;
    Ok(directory)
}

fn open_directory_at(parent: &fs::File, name: &OsStr, label: &str) -> Result<fs::File, String> {
    let component =
        CString::new(name.as_bytes()).map_err(|_| format!("{label} contains an interior NUL"))?;
    open_directory_at_cstr(parent, component.as_c_str(), label)
}

fn open_optional_directory_at(
    parent: &fs::File,
    name: &OsStr,
    label: &str,
) -> Result<Option<fs::File>, String> {
    let component =
        CString::new(name.as_bytes()).map_err(|_| format!("{label} contains an interior NUL"))?;
    let fd = match rustix::fs::openat(
        parent,
        component.as_c_str(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(format!("failed to pin {label}: {error}")),
    };
    let file = fs::File::from(fd);
    directory_identity(&file)?;
    Ok(Some(file))
}

fn open_directory_at_cstr(parent: &fs::File, name: &CStr, label: &str) -> Result<fs::File, String> {
    let fd = rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|error| format!("failed to pin {label}: {error}"))?;
    let file = fs::File::from(fd);
    directory_identity(&file)?;
    Ok(file)
}

fn open_expected_directory_at(
    parent: &fs::File,
    name: &OsStr,
    expected: &NodeIdentity,
    label: &str,
) -> Result<fs::File, String> {
    let component =
        CString::new(name.as_bytes()).map_err(|_| format!("{label} contains an interior NUL"))?;
    open_expected_directory_at_cstr(parent, component.as_c_str(), expected, label)
}

fn open_expected_directory_at_cstr(
    parent: &fs::File,
    name: &CStr,
    expected: &NodeIdentity,
    label: &str,
) -> Result<fs::File, String> {
    let directory = open_directory_at_cstr(parent, name, label)?;
    require_same_mount(parent, &directory, label)?;
    let actual = directory_identity(&directory)?;
    if !same_directory_identity(&actual, expected) {
        return Err(format!("{label} is a replacement"));
    }
    Ok(directory)
}

fn named_directory_state(
    parent: &fs::File,
    name: &OsStr,
    expected: &NodeIdentity,
) -> Result<CoordinateState, String> {
    let component = CString::new(name.as_bytes())
        .map_err(|_| "removal coordinate contains an interior NUL".to_string())?;
    match rustix::fs::statat(parent, component.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) => {
            if !FileType::from_raw_mode(stat.st_mode).is_dir() {
                return Ok(CoordinateState::Replacement);
            }
            let actual =
                match open_directory_at_cstr(parent, component.as_c_str(), "removal coordinate") {
                    Ok(directory) => {
                        if require_same_mount(parent, &directory, "removal coordinate").is_err() {
                            return Ok(CoordinateState::Replacement);
                        }
                        directory_identity(&directory)?
                    }
                    Err(_) => return Ok(CoordinateState::Replacement),
                };
            Ok(if same_directory_identity(&actual, expected) {
                CoordinateState::Expected
            } else {
                CoordinateState::Replacement
            })
        }
        Err(rustix::io::Errno::NOENT) => Ok(CoordinateState::Absent),
        Err(error) => Err(format!("failed to inspect removal coordinate: {error}")),
    }
}

fn verify_named_directory(
    parent: &fs::File,
    name: &OsStr,
    expected: &NodeIdentity,
) -> Result<(), String> {
    match named_directory_state(parent, name, expected)? {
        CoordinateState::Expected => Ok(()),
        CoordinateState::Absent => Err("SchoolX Code removal directory disappeared".to_string()),
        CoordinateState::Replacement => {
            Err("SchoolX Code removal directory was replaced".to_string())
        }
    }
}

fn pin_recovery_boundary(
    authority: &super::super::CodeWorktreeRemovalAuthority,
    stored: &StoredManifest,
    nest_root: &Path,
) -> Result<RecoveryBoundary, String> {
    let canonical_nest = nest_root
        .canonicalize()
        .map_err(|error| format!("failed to resolve SchoolX nest for recovery: {error}"))?;
    if canonical_nest != nest_root {
        return Err("SchoolX Code removal recovery nest is not canonical".to_string());
    }
    let nest = open_directory_absolute(&canonical_nest, "SchoolX nest")?;
    let worktrees = open_directory_at(&nest, OsStr::new("WORKTREES"), "WORKTREES")?;
    require_same_mount(&nest, &worktrees, "SchoolX WORKTREES")?;
    let root_parent = open_expected_directory_at(
        &worktrees,
        OsStr::new(&authority.binding.repository_identity),
        &stored.manifest.root_parent_identity,
        "repository worktree bucket",
    )?;
    let common_dir_path = Path::new(&authority.physical.git_admin_parent)
        .parent()
        .ok_or_else(|| "SchoolX Code removal Git-admin parent has no common dir".to_string())?
        .to_path_buf();
    if repository_identity(&common_dir_path)? != authority.binding.repository_identity {
        return Err("SchoolX Code removal recovery common-dir identity changed".to_string());
    }
    let common_dir = open_directory_absolute(&common_dir_path, "Git common directory")?;
    if !same_directory_identity(
        &directory_identity(&common_dir)?,
        &stored.manifest.common_dir_identity,
    ) {
        return Err("SchoolX Code removal recovery common dir was replaced".to_string());
    }
    let admin_parent = open_expected_directory_at(
        &common_dir,
        OsStr::new("worktrees"),
        &stored.manifest.admin_parent_identity,
        "Git-admin parent",
    )?;
    let root_state = named_directory_state(
        &root_parent,
        worktree_name(authority)?,
        &stored.manifest.root_identity,
    )?;
    let quarantine_state = named_directory_state(
        &root_parent,
        OsStr::new(&authority.physical.quarantine_name),
        &stored.manifest.root_identity,
    )?;
    let root = match (root_state, quarantine_state) {
        (CoordinateState::Expected, CoordinateState::Absent) => Some(open_expected_directory_at(
            &root_parent,
            worktree_name(authority)?,
            &stored.manifest.root_identity,
            "managed removal root",
        )?),
        (CoordinateState::Absent, CoordinateState::Expected) => Some(open_expected_directory_at(
            &root_parent,
            OsStr::new(&authority.physical.quarantine_name),
            &stored.manifest.root_identity,
            "removal quarantine",
        )?),
        _ => None,
    };
    let admin = match named_directory_state(
        &admin_parent,
        OsStr::new(&authority.physical.git_admin_entry),
        &stored.manifest.admin_identity,
    )? {
        CoordinateState::Expected => Some(open_expected_directory_at(
            &admin_parent,
            OsStr::new(&authority.physical.git_admin_entry),
            &stored.manifest.admin_identity,
            "Git-admin entry",
        )?),
        _ => None,
    };
    let git_launch = RemovalGitLaunchAuthority::admit(&common_dir)?;
    Ok(RecoveryBoundary {
        nest,
        worktrees,
        root_parent,
        root,
        common_dir,
        admin_parent,
        admin,
        common_dir_path,
        git_launch,
    })
}

fn verify_recovery_boundary_paths(
    boundary: &RecoveryBoundary,
    authority: &super::super::CodeWorktreeRemovalAuthority,
    stored: &StoredManifest,
) -> Result<(), String> {
    verify_named_directory(
        &boundary.nest,
        OsStr::new("WORKTREES"),
        &directory_identity(&boundary.worktrees)?,
    )?;
    verify_named_directory(
        &boundary.worktrees,
        OsStr::new(&authority.binding.repository_identity),
        &stored.manifest.root_parent_identity,
    )?;
    let current_common =
        open_directory_absolute(&boundary.common_dir_path, "Git common directory")?;
    if !same_directory_identity(
        &directory_identity(&current_common)?,
        &stored.manifest.common_dir_identity,
    ) {
        return Err("SchoolX Code removal common-dir coordinate was replaced".to_string());
    }
    verify_named_directory(
        &current_common,
        OsStr::new("worktrees"),
        &stored.manifest.admin_parent_identity,
    )
}

fn verify_layout_against_authority(
    layout: &PinnedLayout,
    authority: &super::super::CodeWorktreeRemovalAuthority,
    stored: &StoredManifest,
) -> Result<(), String> {
    if path_string(&layout.admin_parent_path, "Git-admin parent")?
        != authority.physical.git_admin_parent
        || layout.admin_entry.as_bytes() != authority.physical.git_admin_entry.as_bytes()
        || stored.digest != authority.physical_manifest_digest
    {
        return Err("SchoolX Code pinned layout does not match removal authority".to_string());
    }
    if !same_directory_identity(
        &directory_identity(&layout.common_dir)?,
        &stored.manifest.common_dir_identity,
    ) {
        return Err("SchoolX Code pinned common dir does not match removal authority".to_string());
    }
    verify_pinned_layout(layout, &authority.binding)?;
    verify_admin_reciprocal(layout, &authority.binding)
}

fn observe_coordinates(
    boundary: &RecoveryBoundary,
    authority: &super::super::CodeWorktreeRemovalAuthority,
    stored: &StoredManifest,
) -> Result<(CoordinateState, CoordinateState, CoordinateState), String> {
    Ok((
        named_directory_state(
            &boundary.root_parent,
            worktree_name(authority)?,
            &stored.manifest.root_identity,
        )?,
        named_directory_state(
            &boundary.root_parent,
            OsStr::new(&authority.physical.quarantine_name),
            &stored.manifest.root_identity,
        )?,
        named_directory_state(
            &boundary.admin_parent,
            OsStr::new(&authority.physical.git_admin_entry),
            &stored.manifest.admin_identity,
        )?,
    ))
}

fn quarantine_root(
    root_parent: &fs::File,
    authority: &super::super::CodeWorktreeRemovalAuthority,
    stored: &StoredManifest,
) -> Result<(), String> {
    verify_named_directory(
        root_parent,
        worktree_name(authority)?,
        &stored.manifest.root_identity,
    )?;
    let source = CString::new(worktree_name(authority)?.as_bytes())
        .map_err(|_| "removal worktree name contains NUL".to_string())?;
    let destination = CString::new(authority.physical.quarantine_name.as_bytes())
        .map_err(|_| "removal quarantine name contains NUL".to_string())?;
    rustix::fs::renameat_with(
        root_parent,
        source.as_c_str(),
        root_parent,
        destination.as_c_str(),
        RenameFlags::NOREPLACE,
    )
    .map_err(|error| {
        format!("failed to quarantine managed worktree without replacement: {error}")
    })?;
    rustix::fs::fsync(root_parent)
        .map_err(|error| format!("failed to sync worktree parent after quarantine: {error}"))?;
    if named_directory_state(
        root_parent,
        OsStr::new(&authority.physical.quarantine_name),
        &stored.manifest.root_identity,
    )? != CoordinateState::Expected
    {
        return Err("quarantined worktree identity changed during rename".to_string());
    }
    Ok(())
}

fn delete_manifest_tree(
    root: &Option<fs::File>,
    entries: &[ManifestEntry],
    root_identity: &NodeIdentity,
    hook: &mut dyn FaultHook,
    worktree: bool,
    path_guard: &mut dyn FnMut() -> Result<(), String>,
) -> Result<(), String> {
    path_guard()?;
    let root = root
        .as_ref()
        .ok_or_else(|| "SchoolX Code removal tree handle is unavailable".to_string())?;
    if !same_directory_identity(&directory_identity(root)?, root_identity) {
        return Err("SchoolX Code removal tree was replaced".to_string());
    }
    verify_manifest_tree_state(root, entries)?;
    let expected = entries
        .iter()
        .map(|entry| Ok((decode_hex_path(&entry.path_hex)?, entry)))
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let mut ordered = expected.iter().collect::<Vec<_>>();
    ordered.sort_by(|(left_path, left), (right_path, right)| {
        let left_depth = left_path.iter().filter(|byte| **byte == b'/').count();
        let right_depth = right_path.iter().filter(|byte| **byte == b'/').count();
        left.kind
            .eq(&ManifestEntryKind::Directory)
            .cmp(&right.kind.eq(&ManifestEntryKind::Directory))
            .then_with(|| right_depth.cmp(&left_depth))
            .then_with(|| right_path.cmp(left_path))
    });
    let deleted_prefix = verify_known_deletion_prefix(root, &ordered, &expected)?;
    for (index, (_, entry)) in ordered.into_iter().enumerate().skip(deleted_prefix) {
        path_guard()?;
        if !manifest_entry_is_present(root, entry, &expected)? {
            return Err("SchoolX Code removal observed a non-prefix manifest deletion".to_string());
        }
        delete_manifest_entry(root, entry, &expected)?;
        rustix::fs::fsync(root).map_err(|error| format!("failed to sync removal tree: {error}"))?;
        hook.after(if worktree {
            FaultBoundary::RootEntryDeleted(index)
        } else {
            FaultBoundary::AdminEntryDeleted(index)
        })?;
        path_guard()?;
    }
    verify_directory_empty(root)
}

fn verify_known_deletion_prefix(
    root: &fs::File,
    ordered: &[(&Vec<u8>, &&ManifestEntry)],
    expected: &BTreeMap<Vec<u8>, &ManifestEntry>,
) -> Result<usize, String> {
    let mut deleted_prefix = 0_usize;
    let mut observed_present = false;
    for (_, entry) in ordered {
        if manifest_entry_is_present(root, entry, expected)? {
            observed_present = true;
        } else if observed_present {
            return Err(
                "SchoolX Code removal manifest has a deletion outside the known prefix".to_string(),
            );
        } else {
            deleted_prefix += 1;
        }
    }
    Ok(deleted_prefix)
}

fn manifest_entry_is_present(
    root: &fs::File,
    entry: &ManifestEntry,
    expected: &BTreeMap<Vec<u8>, &ManifestEntry>,
) -> Result<bool, String> {
    let path = decode_hex_path(&entry.path_hex)?;
    let components = split_relative_bytes(&path)?;
    let (name, ancestors) = components
        .split_last()
        .ok_or_else(|| "SchoolX Code removal manifest path is empty".to_string())?;
    let mut directories = Vec::with_capacity(ancestors.len());
    for (depth, component) in ancestors.iter().enumerate() {
        let parent = directories.last().unwrap_or(root);
        let child_path = join_components(&components[..=depth]);
        let expected_directory = expected.get(&child_path).ok_or_else(|| {
            "SchoolX Code removal manifest is missing an ancestor directory".to_string()
        })?;
        let component = CString::new(component.to_vec())
            .map_err(|_| "SchoolX Code removal ancestor contains NUL".to_string())?;
        match rustix::fs::statat(parent, component.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => return Ok(false),
            Err(error) => {
                return Err(format!(
                    "failed to inspect SchoolX Code removal ancestor: {error}"
                ))
            }
            Ok(_) => {}
        }
        verify_entry_identity(parent, component.as_c_str(), expected_directory)?;
        directories.push(open_expected_directory_at_cstr(
            parent,
            component.as_c_str(),
            &expected_directory.identity,
            "manifest ancestor",
        )?);
    }
    let parent = directories.last().unwrap_or(root);
    let name = CString::new(name.to_vec())
        .map_err(|_| "SchoolX Code removal manifest name contains NUL".to_string())?;
    match rustix::fs::statat(parent, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => Ok(false),
        Err(error) => Err(format!("failed to inspect frozen manifest entry: {error}")),
        Ok(_) => {
            verify_entry_identity(parent, name.as_c_str(), entry)?;
            Ok(true)
        }
    }
}

fn verify_manifest_tree_state(root: &fs::File, entries: &[ManifestEntry]) -> Result<(), String> {
    let expected = entries
        .iter()
        .map(|entry| Ok((decode_hex_path(&entry.path_hex)?, entry)))
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    verify_directory_entries_recursive(root, Vec::new(), &expected)
}

fn verify_directory_entries_recursive(
    directory: &fs::File,
    prefix: Vec<u8>,
    expected: &BTreeMap<Vec<u8>, &ManifestEntry>,
) -> Result<(), String> {
    let mut dir = Dir::read_from(directory)
        .map_err(|error| format!("failed to enumerate removal tree: {error}"))?;
    while let Some(entry) = dir.read() {
        let entry = entry.map_err(|error| format!("failed to read removal tree: {error}"))?;
        let name = entry.file_name().to_bytes();
        if name == b"." || name == b".." {
            continue;
        }
        let path = join_relative(&prefix, name);
        let Some(manifest_entry) = expected.get(&path) else {
            return Err("SchoolX Code removal tree contains a new unmanifested entry".to_string());
        };
        verify_entry_identity(directory, entry.file_name(), manifest_entry)?;
        if manifest_entry.kind == ManifestEntryKind::Directory {
            let child = open_expected_directory_at_cstr(
                directory,
                entry.file_name(),
                &manifest_entry.identity,
                "manifest directory",
            )?;
            verify_directory_entries_recursive(&child, path, expected)?;
        }
    }
    Ok(())
}

fn delete_manifest_entry(
    root: &fs::File,
    entry: &ManifestEntry,
    expected: &BTreeMap<Vec<u8>, &ManifestEntry>,
) -> Result<(), String> {
    let path = decode_hex_path(&entry.path_hex)?;
    let components = split_relative_bytes(&path)?;
    let (name, ancestors) = components
        .split_last()
        .ok_or_else(|| "SchoolX Code removal manifest path is empty".to_string())?;
    let mut directories = Vec::with_capacity(ancestors.len());
    for (depth, component) in ancestors.iter().enumerate() {
        let parent = directories.last().unwrap_or(root);
        let child_path = join_components(&components[..=depth]);
        let expected_directory = expected.get(&child_path).ok_or_else(|| {
            "SchoolX Code removal manifest is missing an ancestor directory".to_string()
        })?;
        if expected_directory.kind != ManifestEntryKind::Directory {
            return Err("SchoolX Code removal manifest ancestor is not a directory".to_string());
        }
        let component_c = CString::new(component.to_vec())
            .map_err(|_| "SchoolX Code removal ancestor contains NUL".to_string())?;
        match rustix::fs::statat(parent, component_c.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => return Ok(()),
            Err(error) => {
                return Err(format!(
                    "failed to inspect SchoolX Code removal ancestor: {error}"
                ))
            }
            Ok(_) => {}
        }
        verify_entry_identity(parent, component_c.as_c_str(), expected_directory)?;
        let child = open_expected_directory_at_cstr(
            parent,
            component_c.as_c_str(),
            &expected_directory.identity,
            "manifest ancestor",
        )?;
        directories.push(child);
    }
    let parent = directories.last().unwrap_or(root);
    let name = CString::new(name.to_vec())
        .map_err(|_| "SchoolX Code removal manifest name contains NUL".to_string())?;
    match rustix::fs::statat(parent, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => return Ok(()),
        Err(error) => {
            return Err(format!(
                "failed to inspect manifest deletion entry: {error}"
            ))
        }
        Ok(_) => {}
    }
    verify_entry_identity(parent, name.as_c_str(), entry)?;
    let flags = if entry.kind == ManifestEntryKind::Directory {
        AtFlags::REMOVEDIR
    } else {
        AtFlags::empty()
    };
    rustix::fs::unlinkat(parent, name.as_c_str(), flags)
        .map_err(|error| format!("failed to remove exact manifest entry: {error}"))?;
    rustix::fs::fsync(parent)
        .map_err(|error| format!("failed to sync exact manifest parent: {error}"))
}

fn verify_entry_identity(
    parent: &fs::File,
    name: &CStr,
    entry: &ManifestEntry,
) -> Result<(), String> {
    let stat = rustix::fs::statat(parent, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|error| format!("failed to inspect frozen manifest entry: {error}"))?;
    let actual = match entry.kind {
        ManifestEntryKind::Directory => {
            if !FileType::from_raw_mode(stat.st_mode).is_dir() {
                return Err("SchoolX Code manifest directory changed type".to_string());
            }
            let directory = open_directory_at_cstr(parent, name, "frozen manifest directory")?;
            require_same_mount(parent, &directory, "frozen manifest directory")?;
            directory_identity(&directory)?
        }
        ManifestEntryKind::GitFile
        | ManifestEntryKind::RegularFile
        | ManifestEntryKind::AdminFile => read_regular_identity_at(parent, name, &stat)?,
        ManifestEntryKind::Symlink => read_symlink_identity_at(parent, name, &stat)?,
    };
    let unchanged = if entry.kind == ManifestEntryKind::Directory {
        same_directory_identity(&actual, &entry.identity)
            && actual.content_sha256.is_none()
            && entry.identity.content_sha256.is_none()
    } else {
        same_named_identity(&actual, &entry.identity)
    };
    if !unchanged {
        return Err("SchoolX Code manifest entry was replaced or changed".to_string());
    }
    Ok(())
}

fn verify_directory_empty(directory: &fs::File) -> Result<(), String> {
    let mut dir = Dir::read_from(directory)
        .map_err(|error| format!("failed to enumerate removal directory: {error}"))?;
    while let Some(entry) = dir.read() {
        let entry = entry.map_err(|error| format!("failed to read removal directory: {error}"))?;
        if !matches!(entry.file_name().to_bytes(), b"." | b"..") {
            return Err("SchoolX Code removal directory gained an unexpected entry".to_string());
        }
    }
    Ok(())
}

fn remove_named_root(
    parent: &fs::File,
    name: &OsStr,
    expected: &NodeIdentity,
) -> Result<(), String> {
    verify_named_directory(parent, name, expected)?;
    let component = CString::new(name.as_bytes())
        .map_err(|_| "removal directory name contains NUL".to_string())?;
    rustix::fs::unlinkat(parent, component.as_c_str(), AtFlags::REMOVEDIR)
        .map_err(|error| format!("failed to remove exact empty removal directory: {error}"))?;
    rustix::fs::fsync(parent).map_err(|error| format!("failed to sync removal parent: {error}"))
}

fn verify_final_absence_and_siblings(
    boundary: &RecoveryBoundary,
    authority: &super::super::CodeWorktreeRemovalAuthority,
    stored: &StoredManifest,
) -> Result<(), String> {
    if observe_coordinates(boundary, authority, stored)?
        != (
            CoordinateState::Absent,
            CoordinateState::Absent,
            CoordinateState::Absent,
        )
    {
        return Err("SchoolX Code removal could not verify exact physical absence".to_string());
    }
    if !same_directory_identity(
        &directory_identity(&boundary.root_parent)?,
        &stored.manifest.root_parent_identity,
    ) || !same_directory_identity(
        &directory_identity(&boundary.admin_parent)?,
        &stored.manifest.admin_parent_identity,
    ) {
        return Err("SchoolX Code removal parent identity changed".to_string());
    }
    let worktree_id = worktree_name(authority)?.as_bytes();
    verify_sibling_snapshot(
        &boundary.root_parent,
        &stored.manifest.root_parent_siblings,
        &[worktree_id, authority.physical.quarantine_name.as_bytes()],
        None,
    )?;
    verify_sibling_snapshot(
        &boundary.admin_parent,
        &stored.manifest.admin_parent_siblings,
        &[authority.physical.git_admin_entry.as_bytes()],
        None,
    )?;
    rustix::fs::fsync(&boundary.root_parent)
        .map_err(|error| format!("failed to sync removed worktree parent: {error}"))?;
    rustix::fs::fsync(&boundary.admin_parent)
        .map_err(|error| format!("failed to sync removed Git-admin parent: {error}"))?;
    Ok(())
}

fn parse_gitdir_file(bytes: &[u8], root: &Path) -> Result<PathBuf, String> {
    let value = bytes
        .strip_prefix(b"gitdir: ")
        .ok_or_else(|| "linked-worktree .git has an invalid prefix".to_string())?;
    let value = trim_one_line(value, "linked-worktree .git")?;
    let path = PathBuf::from(OsString::from_vec(value.to_vec()));
    Ok(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}

fn parse_plain_path_file(bytes: &[u8], parent: &Path) -> Result<PathBuf, String> {
    let value = trim_one_line(bytes, "Git-admin path file")?;
    let path = PathBuf::from(OsString::from_vec(value.to_vec()));
    Ok(if path.is_absolute() {
        path
    } else {
        parent.join(path)
    })
}

fn trim_one_line<'a>(bytes: &'a [u8], label: &str) -> Result<&'a [u8], String> {
    let value = bytes
        .strip_suffix(b"\r\n")
        .or_else(|| bytes.strip_suffix(b"\n"))
        .unwrap_or(bytes);
    if value.is_empty()
        || value.contains(&b'\n')
        || value.contains(&b'\r')
        || value.contains(&b'\0')
    {
        return Err(format!("{label} does not contain one safe path"));
    }
    Ok(value)
}

fn worktree_name(authority: &super::super::CodeWorktreeRemovalAuthority) -> Result<&OsStr, String> {
    let worktree_id = authority
        .binding
        .worktree_id
        .as_deref()
        .ok_or_else(|| "SchoolX Code removal authority lost its worktree id".to_string())?;
    Ok(OsStr::new(worktree_id))
}

fn validate_safe_component(value: &OsStr, label: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes == b"."
        || bytes == b".."
        || bytes.contains(&b'/')
        || bytes.contains(&b'\0')
    {
        return Err(format!("SchoolX Code {label} is not one safe component"));
    }
    Ok(())
}

fn validate_relative_bytes(path: &[u8]) -> Result<(), String> {
    if path.is_empty() || path.starts_with(b"/") || path.ends_with(b"/") || path.contains(&b'\0') {
        return Err("SchoolX Code removal path is not repository-relative".to_string());
    }
    let components = path.split(|byte| *byte == b'/').collect::<Vec<_>>();
    if components
        .iter()
        .any(|component| component.is_empty() || *component == b"." || *component == b"..")
    {
        return Err("SchoolX Code removal path contains an unsafe component".to_string());
    }
    Ok(())
}

fn split_relative_bytes(path: &[u8]) -> Result<Vec<&[u8]>, String> {
    validate_relative_bytes(path)?;
    Ok(path.split(|byte| *byte == b'/').collect())
}

fn join_relative(prefix: &[u8], name: &[u8]) -> Vec<u8> {
    if prefix.is_empty() {
        return name.to_vec();
    }
    let mut result = Vec::with_capacity(prefix.len() + 1 + name.len());
    result.extend_from_slice(prefix);
    result.push(b'/');
    result.extend_from_slice(name);
    result
}

fn join_components(components: &[&[u8]]) -> Vec<u8> {
    let total = components
        .iter()
        .map(|component| component.len())
        .sum::<usize>()
        + components.len().saturating_sub(1);
    let mut result = Vec::with_capacity(total);
    for (index, component) in components.iter().enumerate() {
        if index != 0 {
            result.push(b'/');
        }
        result.extend_from_slice(component);
    }
    result
}

fn decode_hex_path(value: &str) -> Result<Vec<u8>, String> {
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return Err("SchoolX Code removal manifest path encoding is invalid".to_string());
    }
    hex::decode(value).map_err(|error| format!("invalid removal manifest path encoding: {error}"))
}

fn sort_manifest_entries(entries: &mut [ManifestEntry]) {
    entries.sort_by(|left, right| left.path_hex.cmp(&right.path_hex));
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn path_string(path: &Path, label: &str) -> Result<String, String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{label} is not valid UTF-8"))
}

fn zero_oid_for(commit: &str) -> Result<&'static str, String> {
    match commit.len() {
        40 => Ok(ZERO_SHA1),
        64 => Ok(ZERO_SHA256),
        _ => Err("SchoolX Code removal proof has an unsupported object-id length".to_string()),
    }
}

fn read_proof_ref(
    launch: &RemovalGitLaunchAuthority,
    common_dir: &fs::File,
    common_dir_path: &Path,
    authority: &super::super::CodeWorktreeRemovalAuthority,
    deadline: Instant,
) -> Result<Option<String>, String> {
    let captured = run_helper(
        launch,
        common_dir,
        RemovalGitRequest::ReadProofRef {
            git_executable: launch.executable_string()?,
            expected_target_path: path_string(common_dir_path, "removal common dir")?,
            removal_id: authority.removal_id.clone(),
        },
        deadline,
    )?;
    match captured.status.code() {
        Some(0) if captured.stderr.bytes.is_empty() => {
            if captured.stdout.bytes.is_empty() {
                return Ok(None);
            }
            let value = one_line(&captured.stdout.bytes, "removal proof ref")?;
            let mut fields = value.split('\t');
            let name = fields
                .next()
                .ok_or_else(|| "removal proof ref output omitted its name".to_string())?;
            let object_id = fields
                .next()
                .ok_or_else(|| "removal proof ref output omitted its object id".to_string())?;
            let symbolic_target = fields.next().ok_or_else(|| {
                "removal proof ref output omitted its symbolic target".to_string()
            })?;
            if fields.next().is_some()
                || name != proof_ref_from_id(&authority.removal_id)
                || !symbolic_target.is_empty()
            {
                return Err(
                    "SchoolX Code removal proof ref is symbolic or has ambiguous raw authority"
                        .to_string(),
                );
            }
            validate_commit_id(object_id)?;
            Ok(Some(object_id.to_string()))
        }
        _ => Err(captured_error("removal proof-ref read", &captured)),
    }
}

fn ensure_proof_ref(
    launch: &RemovalGitLaunchAuthority,
    common_dir: &fs::File,
    common_dir_path: &Path,
    authority: &super::super::CodeWorktreeRemovalAuthority,
    deadline: Instant,
) -> Result<(), String> {
    match read_proof_ref(launch, common_dir, common_dir_path, authority, deadline)? {
        Some(current) if current == authority.merge_proof.target_commit => {
            return sync_exact_proof_ref(common_dir, authority)
        }
        Some(_) => {
            return Err("SchoolX Code removal proof ref is owned by a replacement".to_string())
        }
        None => {}
    }
    let captured = run_helper(
        launch,
        common_dir,
        RemovalGitRequest::CreateProofRef {
            git_executable: launch.executable_string()?,
            expected_target_path: path_string(common_dir_path, "removal common dir")?,
            removal_id: authority.removal_id.clone(),
            target_commit: authority.merge_proof.target_commit.clone(),
            zero_oid: zero_oid_for(&authority.merge_proof.target_commit)?.to_string(),
        },
        deadline,
    )?;
    require_success(&captured, "removal proof-ref create")?;
    require_exact_proof_ref(launch, common_dir, common_dir_path, authority, deadline)
}

fn require_exact_proof_ref(
    launch: &RemovalGitLaunchAuthority,
    common_dir: &fs::File,
    common_dir_path: &Path,
    authority: &super::super::CodeWorktreeRemovalAuthority,
    deadline: Instant,
) -> Result<(), String> {
    match read_proof_ref(launch, common_dir, common_dir_path, authority, deadline)? {
        Some(current) if current == authority.merge_proof.target_commit => {
            sync_exact_proof_ref(common_dir, authority)
        }
        Some(_) => Err("SchoolX Code removal proof ref was replaced".to_string()),
        None => Err("SchoolX Code removing state lost its exact proof ref".to_string()),
    }
}

fn delete_proof_ref_if_matches(
    launch: &RemovalGitLaunchAuthority,
    common_dir: &fs::File,
    common_dir_path: &Path,
    authority: &super::super::CodeWorktreeRemovalAuthority,
    deadline: Instant,
) -> Result<(), String> {
    match read_proof_ref(launch, common_dir, common_dir_path, authority, deadline)? {
        None => return sync_deleted_proof_ref(common_dir, authority),
        Some(current) if current == authority.merge_proof.target_commit => {
            sync_exact_proof_ref(common_dir, authority)?;
        }
        Some(_) => {
            return Err(
                "SchoolX Code removal proof ref replacement was preserved during cleanup"
                    .to_string(),
            )
        }
    }
    let captured = run_helper(
        launch,
        common_dir,
        RemovalGitRequest::DeleteProofRef {
            git_executable: launch.executable_string()?,
            expected_target_path: path_string(common_dir_path, "removal common dir")?,
            removal_id: authority.removal_id.clone(),
            target_commit: authority.merge_proof.target_commit.clone(),
        },
        deadline,
    )?;
    require_success(&captured, "removal proof-ref compare-delete")?;
    match read_proof_ref(launch, common_dir, common_dir_path, authority, deadline)? {
        None => sync_deleted_proof_ref(common_dir, authority),
        Some(_) => Err("SchoolX Code removal proof ref remained after cleanup".to_string()),
    }
}

fn sync_exact_proof_ref(
    common_dir: &fs::File,
    authority: &super::super::CodeWorktreeRemovalAuthority,
) -> Result<(), String> {
    let refs = open_directory_at(common_dir, OsStr::new("refs"), "Git refs directory")?;
    require_same_mount(common_dir, &refs, "Git refs directory")?;
    let schoolx = open_directory_at(&refs, OsStr::new("schoolx"), "SchoolX ref directory")?;
    require_same_mount(&refs, &schoolx, "SchoolX ref directory")?;
    let claims = open_directory_at(
        &schoolx,
        OsStr::new("removal-claims"),
        "SchoolX removal-ref directory",
    )?;
    require_same_mount(&schoolx, &claims, "SchoolX removal-ref directory")?;
    let ref_file = read_small_regular_at(
        &claims,
        OsStr::new(&authority.removal_id),
        256,
        "SchoolX removal proof ref",
    )?;
    if one_line(&ref_file.bytes, "SchoolX removal proof ref")?
        != authority.merge_proof.target_commit
    {
        return Err("SchoolX Code removal loose proof ref changed".to_string());
    }
    let component = CString::new(authority.removal_id.as_bytes())
        .map_err(|_| "SchoolX Code removal proof ref contains NUL".to_string())?;
    let fd = rustix::fs::openat(
        &claims,
        component.as_c_str(),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|error| format!("failed to pin SchoolX removal proof ref: {error}"))?;
    let file = fs::File::from(fd);
    file.sync_all()
        .map_err(|error| format!("failed to sync SchoolX removal proof ref: {error}"))?;
    for (directory, label) in [
        (&claims, "removal-ref directory"),
        (&schoolx, "SchoolX ref directory"),
        (&refs, "Git refs directory"),
        (common_dir, "Git common directory"),
    ] {
        rustix::fs::fsync(directory).map_err(|error| format!("failed to sync {label}: {error}"))?;
    }
    Ok(())
}

fn sync_deleted_proof_ref(
    common_dir: &fs::File,
    authority: &super::super::CodeWorktreeRemovalAuthority,
) -> Result<(), String> {
    let refs = open_directory_at(common_dir, OsStr::new("refs"), "Git refs directory")?;
    require_same_mount(common_dir, &refs, "Git refs directory")?;
    let schoolx =
        open_optional_directory_at(&refs, OsStr::new("schoolx"), "SchoolX ref directory")?;
    let claims = match schoolx.as_ref() {
        Some(schoolx) => {
            require_same_mount(&refs, schoolx, "SchoolX ref directory")?;
            open_optional_directory_at(
                schoolx,
                OsStr::new("removal-claims"),
                "SchoolX removal-ref directory",
            )?
        }
        None => None,
    };
    if let Some(claims) = claims.as_ref() {
        let component = CString::new(authority.removal_id.as_bytes())
            .map_err(|_| "SchoolX Code removal proof ref contains NUL".to_string())?;
        match rustix::fs::statat(claims, component.as_c_str(), AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => {}
            Ok(_) => return Err("SchoolX Code removal loose proof ref remained".to_string()),
            Err(error) => {
                return Err(format!(
                    "failed to verify deleted SchoolX removal proof ref: {error}"
                ))
            }
        }
        let schoolx = schoolx
            .as_ref()
            .ok_or_else(|| "SchoolX removal-ref parent disappeared".to_string())?;
        require_same_mount(schoolx, claims, "SchoolX removal-ref directory")?;
        rustix::fs::fsync(claims)
            .map_err(|error| format!("failed to sync removal-ref directory: {error}"))?;
    }
    if let Some(schoolx) = schoolx.as_ref() {
        rustix::fs::fsync(schoolx)
            .map_err(|error| format!("failed to sync SchoolX ref directory: {error}"))?;
    }
    rustix::fs::fsync(&refs)
        .map_err(|error| format!("failed to sync Git refs directory: {error}"))?;
    rustix::fs::fsync(common_dir)
        .map_err(|error| format!("failed to sync Git common directory: {error}"))
}

#[cfg(test)]
pub(super) fn execute_helper() -> Result<(), String> {
    let encoded = std::env::var(HELPER_ENV)
        .map_err(|_| "removal Git helper request was missing or not UTF-8".to_string())?;
    if encoded.len() > MAX_HELPER_REQUEST_BYTES {
        return Err(format!(
            "removal Git helper request exceeds {MAX_HELPER_REQUEST_BYTES} bytes"
        ));
    }
    let envelope: RemovalGitEnvelope = serde_json::from_str(&encoded)
        .map_err(|error| format!("invalid removal Git helper request: {error}"))?;
    validate_helper_envelope(&envelope)?;
    let stdin = std::io::stdin();
    let stat = rustix::fs::fstat(stdin.as_fd())
        .map_err(|error| format!("failed to inspect removal Git helper directory: {error}"))?;
    if !FileType::from_raw_mode(stat.st_mode).is_dir()
        || stat.st_dev as u64 != envelope.target_device
        || stat.st_ino as u64 != envelope.target_inode
    {
        return Err("removal Git helper directory identity did not match".to_string());
    }
    rustix::process::fchdir(stdin.as_fd())
        .map_err(|error| format!("failed to enter removal Git helper directory: {error}"))?;
    let current = fs::metadata(".")
        .map_err(|error| format!("failed to verify removal Git helper cwd: {error}"))?;
    if current.dev() != envelope.target_device || current.ino() != envelope.target_inode {
        return Err("removal Git helper changed to a different directory".to_string());
    }
    let mut command = helper_git_command(&envelope.request)?;
    if let RemovalGitRequest::BlobTypes { object_ids, .. } = &envelope.request {
        command.stdin(Stdio::piped());
        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to start removal blob inspector: {error}"))?;
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| "removal blob inspector did not expose stdin".to_string())?;
        for object_id in object_ids {
            writeln!(input, "{object_id}")
                .map_err(|error| format!("failed to write removal blob request: {error}"))?;
        }
        drop(input);
        let status = child
            .wait()
            .map_err(|error| format!("failed to wait for removal blob inspector: {error}"))?;
        if !status.success() {
            return Err(format!(
                "removal blob inspector exited with status {status}"
            ));
        }
        // The helper normally replaces itself with Git. This one operation
        // needs a pipe, so terminate the isolated helper after Git succeeds;
        // returning would let the Rust test harness append bytes to stdout.
        std::process::exit(0);
    }
    let error = command.exec();
    Err(format!("failed to execute removal Git helper: {error}"))
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

#[cfg(target_os = "linux")]
fn run_direct_git(
    launch: &RemovalGitLaunchAuthority,
    target: &fs::File,
    request: &RemovalGitRequest,
    deadline: Instant,
) -> Result<CapturedChild, String> {
    let authority = &launch.direct;
    let timeout = remaining_timeout(deadline)?;
    if Path::new(request.git_executable()) != authority.path() {
        return Err(
            "removal Git request did not match its root-trusted launch authority".to_string(),
        );
    }

    let (stdin, input) = match request {
        RemovalGitRequest::BlobTypes { object_ids, .. } => {
            let (reader, writer) = std::io::pipe()
                .map_err(|error| format!("failed to create removal blob input pipe: {error}"))?;
            let mut payload = Vec::with_capacity(object_ids.len().saturating_mul(66));
            for object_id in object_ids {
                payload.extend_from_slice(object_id.as_bytes());
                payload.push(b'\n');
            }
            (Stdio::from(reader), Some((writer, payload)))
        }
        _ => (Stdio::null(), None),
    };
    let mut command = authority.command();
    configure_helper_git_command(command.command_mut(), request)?;
    let mut child = authority.spawn(target, command, stdin)?;
    let input_writer = input.map(|(mut writer, payload)| {
        std::thread::spawn(move || {
            writer
                .write_all(&payload)
                .map_err(|error| format!("failed to write removal blob request: {error}"))
        })
    });
    let captured = capture_child(&mut child, timeout);
    let written = match input_writer {
        Some(writer) => writer
            .join()
            .map_err(|_| "removal blob input writer panicked".to_string())?,
        None => Ok(()),
    };
    match (captured, written) {
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Ok(captured), Ok(())) => Ok(captured),
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn strip_removal_test_harness_output(output: Vec<u8>) -> Vec<u8> {
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

#[cfg(all(test, target_os = "linux"))]
#[test]
fn linux_onnxruntime_startup_diagnostic_filter_is_exact() {
    let diagnostic =
        b"onnxruntime cpuid_info warning: Unknown CPU vendor. cpuinfo_vendor value: 15\n";
    assert!(
        crate::code_workspace::worktrees::strip_linux_onnxruntime_startup_diagnostic(
            diagnostic.to_vec()
        )
        .is_empty()
    );
    assert!(
        crate::code_workspace::worktrees::strip_linux_onnxruntime_startup_diagnostic(
            b"onnxruntime cpuid_info warning: Unknown CPU vendor. cpuinfo_vendor value: 0".to_vec(),
        )
        .is_empty()
    );

    let followed_by_error = [diagnostic.as_slice(), b"fatal: protected helper error\n"].concat();
    assert_eq!(
        crate::code_workspace::worktrees::strip_linux_onnxruntime_startup_diagnostic(
            followed_by_error
        ),
        b"fatal: protected helper error\n"
    );
    assert_eq!(
        crate::code_workspace::worktrees::strip_linux_onnxruntime_startup_diagnostic(
            b"onnxruntime cpuid_info warning: Unknown CPU vendor. cpuinfo_vendor value: forged\n"
                .to_vec(),
        ),
        b"onnxruntime cpuid_info warning: Unknown CPU vendor. cpuinfo_vendor value: forged\n"
    );
}

fn validate_helper_envelope(envelope: &RemovalGitEnvelope) -> Result<(), String> {
    if envelope.version != HELPER_VERSION {
        return Err(format!(
            "unsupported removal Git helper version {}",
            envelope.version
        ));
    }
    let expected = envelope.request.expected_target_path();
    if !expected.is_absolute() {
        return Err("removal Git helper target must be absolute".to_string());
    }
    let git = Path::new(envelope.request.git_executable());
    if !git.is_absolute() {
        return Err("removal Git helper executable must be absolute".to_string());
    }
    let canonical_git = git
        .canonicalize()
        .map_err(|error| format!("failed to resolve removal Git executable: {error}"))?;
    if canonical_git != git || !canonical_git.is_file() {
        return Err("removal Git helper executable is not canonical".to_string());
    }
    match &envelope.request {
        RemovalGitRequest::Status {
            disabled_filter_keys,
            ..
        } => validate_filter_keys(disabled_filter_keys),
        RemovalGitRequest::ReadProofRef { removal_id, .. } => {
            validate_native_removal_id(removal_id)
        }
        RemovalGitRequest::CreateProofRef {
            removal_id,
            target_commit,
            zero_oid,
            ..
        } => {
            validate_native_removal_id(removal_id)?;
            validate_commit_id(target_commit)?;
            if zero_oid != zero_oid_for(target_commit)? {
                return Err("removal Git helper zero object id is invalid".to_string());
            }
            Ok(())
        }
        RemovalGitRequest::DeleteProofRef {
            removal_id,
            target_commit,
            ..
        } => {
            validate_native_removal_id(removal_id)?;
            validate_commit_id(target_commit)
        }
        RemovalGitRequest::HeadEntries { head_commit, .. } => validate_commit_id(head_commit),
        RemovalGitRequest::BlobTypes { object_ids, .. } => {
            if object_ids.is_empty() || object_ids.len() > MAX_OBJECT_TYPE_BATCH {
                return Err("removal Git helper blob batch has an invalid size".to_string());
            }
            let mut previous = None;
            for object_id in object_ids {
                validate_commit_id(object_id)?;
                if previous.is_some_and(|value: &String| value >= object_id) {
                    return Err(
                        "removal Git helper blob batch must be strictly ordered".to_string()
                    );
                }
                previous = Some(object_id);
            }
            Ok(())
        }
        RemovalGitRequest::LocalConfig { .. }
        | RemovalGitRequest::WorktreeConfigNames { .. }
        | RemovalGitRequest::IndexEntries { .. }
        | RemovalGitRequest::RefFormat { .. } => Ok(()),
    }
}

/// Decode and revalidate one closed removal-Git envelope inside the signed
/// macOS service, then derive its fixed `/usr/bin/git` process specification.
#[cfg(target_os = "macos")]
pub(in crate::code_workspace) fn prepare_macos_removal_git(
    payload: &str,
    cwd: DescriptorObservation,
    stdin: DescriptorObservation,
) -> Result<MacGitProcessSpec, String> {
    if payload.len() > MAX_HELPER_REQUEST_BYTES {
        return Err(format!(
            "removal Git helper request exceeds {MAX_HELPER_REQUEST_BYTES} bytes"
        ));
    }
    let envelope: RemovalGitEnvelope = serde_json::from_str(payload)
        .map_err(|error| format!("invalid removal Git helper request: {error}"))?;
    validate_helper_envelope(&envelope)?;
    macos_git_xpc::validate_directory_observation(
        cwd,
        envelope.target_device,
        envelope.target_inode,
        None,
        "removal Git cwd",
    )?;
    if envelope.request.git_executable() != MACOS_SYSTEM_GIT {
        return Err("macOS removal Git request did not select /usr/bin/git".to_string());
    }
    let trusted_git = crate::code_workspace::git_write::macos_root_trusted_git()?;
    match &envelope.request {
        RemovalGitRequest::BlobTypes { object_ids, .. } => {
            let expected_size = object_ids.iter().try_fold(0_u64, |total, object_id| {
                total
                    .checked_add(object_id.len() as u64 + 1)
                    .ok_or_else(|| "removal blob input size overflowed".to_string())
            })?;
            macos_git_xpc::validate_bounded_regular_observation(
                stdin,
                expected_size,
                "removal blob input",
            )?;
            if stdin.size != expected_size {
                return Err("macOS removal blob input size did not match its request".to_string());
            }
        }
        _ => macos_git_xpc::validate_null_observation(stdin, "removal Git input")?,
    }
    let mut command = Command::new(trusted_git);
    configure_helper_git_command(&mut command, &envelope.request)?;
    macos_git_xpc::process_spec_from_command(&command)
}

#[cfg(all(test, target_os = "macos"))]
mod macos_xpc_tests {
    use super::*;

    #[test]
    fn prepare_revalidates_removal_git_descriptors() -> Result<(), String> {
        if rustix::process::geteuid().as_raw() == 0 {
            return Ok(());
        }
        let target = tempfile::tempdir().map_err(|error| error.to_string())?;
        let metadata = target
            .path()
            .metadata()
            .map_err(|error| error.to_string())?;
        let envelope = RemovalGitEnvelope {
            version: HELPER_VERSION,
            target_device: metadata.dev(),
            target_inode: metadata.ino(),
            request: RemovalGitRequest::LocalConfig {
                git_executable: MACOS_SYSTEM_GIT.to_string(),
                expected_target_path: path_string(target.path(), "test target")?,
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
        prepare_macos_removal_git(&payload, cwd, stdin)?;
        assert!(prepare_macos_removal_git(
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
}

#[cfg(test)]
fn helper_git_command(request: &RemovalGitRequest) -> Result<Command, String> {
    let git = Path::new(request.git_executable());
    let mut command = Command::new(git);
    configure_helper_git_command(&mut command, request)?;
    Ok(command)
}

fn configure_helper_git_command(
    command: &mut Command,
    request: &RemovalGitRequest,
) -> Result<(), String> {
    command.arg("--no-pager");
    let (mutating, filters): (bool, &[String]) = match request {
        RemovalGitRequest::LocalConfig { .. } => {
            command.args(["config", "--local", "--includes", "--null", "--list"]);
            (false, &[])
        }
        RemovalGitRequest::WorktreeConfigNames { .. } => {
            command.args([
                "config",
                "--worktree",
                "--includes",
                "--null",
                "--name-only",
                "--list",
            ]);
            (false, &[])
        }
        RemovalGitRequest::IndexEntries { .. } => {
            command.args([
                "--literal-pathspecs",
                "ls-files",
                "--cached",
                "--stage",
                "-z",
                "--",
            ]);
            (false, &[])
        }
        RemovalGitRequest::HeadEntries { head_commit, .. } => {
            command.args([
                "--literal-pathspecs",
                "ls-tree",
                "-r",
                "-z",
                "--full-tree",
                head_commit,
                "--",
            ]);
            (false, &[])
        }
        RemovalGitRequest::BlobTypes { .. } => {
            command
                .arg("cat-file")
                .arg("--batch-check=%(objectname) %(objecttype)");
            (false, &[])
        }
        RemovalGitRequest::RefFormat { .. } => {
            command.args([
                "config",
                "--local",
                "--get",
                "--default",
                "files",
                "extensions.refStorage",
            ]);
            (false, &[])
        }
        RemovalGitRequest::Status {
            disabled_filter_keys,
            ..
        } => {
            command.args(["status", "--porcelain=v1", "-z", "--untracked-files=all"]);
            (false, disabled_filter_keys)
        }
        RemovalGitRequest::ReadProofRef { removal_id, .. } => {
            let proof_ref = proof_ref_from_id(removal_id);
            command.args([
                "--git-dir=.",
                "for-each-ref",
                "--format=%(refname)\t%(objectname)\t%(symref)",
                "--count=2",
                &proof_ref,
            ]);
            (false, &[])
        }
        RemovalGitRequest::CreateProofRef {
            removal_id,
            target_commit,
            zero_oid,
            ..
        } => {
            command.args([
                "--git-dir=.",
                "update-ref",
                "--no-deref",
                &proof_ref_from_id(removal_id),
                target_commit,
                zero_oid,
            ]);
            (true, &[])
        }
        RemovalGitRequest::DeleteProofRef {
            removal_id,
            target_commit,
            ..
        } => {
            command.args([
                "--git-dir=.",
                "update-ref",
                "--no-deref",
                "-d",
                &proof_ref_from_id(removal_id),
                target_commit,
            ]);
            (true, &[])
        }
    };
    configure_git_environment(command, mutating, filters);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    Ok(())
}

fn configure_git_environment(command: &mut Command, mutating: bool, filters: &[String]) {
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
    command
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_GRAFT_FILE", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_OPTIONAL_LOCKS", if mutating { "1" } else { "0" });
    let mut static_config = vec![
        ("credential.helper", ""),
        ("advice.graftFileDeprecated", "false"),
        ("core.hooksPath", "/dev/null"),
        ("core.fsmonitor", "false"),
        ("protocol.allow", "never"),
        ("core.logAllRefUpdates", "false"),
    ];
    if mutating {
        static_config.push(("core.fsync", "reference"));
        static_config.push(("core.fsyncMethod", "fsync"));
    }
    command.env(
        "GIT_CONFIG_COUNT",
        (static_config.len() + filters.len()).to_string(),
    );
    for (index, &(key, value)) in static_config.iter().enumerate() {
        command.env(format!("GIT_CONFIG_KEY_{index}"), key);
        command.env(format!("GIT_CONFIG_VALUE_{index}"), value);
    }
    for (offset, key) in filters.iter().enumerate() {
        let index = static_config.len() + offset;
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

fn validate_filter_keys(keys: &[String]) -> Result<(), String> {
    if keys.len() > MAX_FILTER_KEYS {
        return Err(format!(
            "removal Git helper filter keys exceed {MAX_FILTER_KEYS}"
        ));
    }
    for key in keys {
        let lower = key.to_ascii_lowercase();
        if key.len() > 4096
            || key.chars().any(char::is_control)
            || !lower.starts_with("filter.")
            || ![".clean", ".smudge", ".process", ".required"]
                .iter()
                .any(|suffix| lower.ends_with(suffix))
        {
            return Err("removal Git helper filter key is invalid".to_string());
        }
    }
    Ok(())
}

fn validate_native_removal_id(value: &str) -> Result<(), String> {
    let parsed = uuid::Uuid::parse_str(value)
        .map_err(|error| format!("removal Git helper id is invalid: {error}"))?;
    if parsed.get_version() != Some(uuid::Version::Random)
        || parsed.hyphenated().to_string() != value
    {
        return Err("removal Git helper id must be a canonical UUID v4".to_string());
    }
    Ok(())
}

fn proof_ref_from_id(removal_id: &str) -> String {
    format!("{PROOF_REF_PREFIX}{removal_id}")
}

fn remaining_timeout(deadline: Instant) -> Result<Duration, String> {
    let timeout = deadline
        .saturating_duration_since(Instant::now())
        .min(GIT_TIMEOUT);
    if timeout.is_zero() {
        return Err("SchoolX Code removal Git budget was exhausted".to_string());
    }
    Ok(timeout)
}

#[cfg(any(not(target_os = "macos"), test))]
fn capture_child(child: &mut Child, timeout: Duration) -> Result<CapturedChild, String> {
    let stdout = spawn_pipe_reader(child.stdout.take());
    let stderr = spawn_pipe_reader(child.stderr.take());
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = crate::managed_agents::terminate_process(child.id());
                let _ = child.kill();
                let _ = child.wait();
                let _ = join_pipe(stdout);
                let _ = join_pipe(stderr);
                return Err("SchoolX Code removal Git helper timed out".to_string());
            }
            Err(error) => {
                let _ = crate::managed_agents::terminate_process(child.id());
                let _ = child.kill();
                let _ = child.wait();
                let _ = join_pipe(stdout);
                let _ = join_pipe(stderr);
                return Err(format!("failed to wait for removal Git helper: {error}"));
            }
        }
    };
    Ok(CapturedChild {
        status,
        stdout: join_pipe(stdout)?,
        stderr: join_pipe(stderr)?,
    })
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

fn spawn_pipe_reader<R>(pipe: Option<R>) -> JoinHandle<CapturedPipe>
where
    R: Read + Send + 'static,
{
    std::thread::spawn(move || {
        let Some(mut pipe) = pipe else {
            return CapturedPipe {
                bytes: Vec::new(),
                truncated: false,
            };
        };
        let mut bytes = Vec::new();
        let mut truncated = false;
        let mut buffer = [0_u8; 8192];
        loop {
            let read = match pipe.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            let remaining = MAX_GIT_OUTPUT_BYTES.saturating_sub(bytes.len());
            let retained = remaining.min(read);
            bytes.extend_from_slice(&buffer[..retained]);
            truncated |= retained < read;
        }
        CapturedPipe { bytes, truncated }
    })
}

fn join_pipe(handle: JoinHandle<CapturedPipe>) -> Result<CapturedPipe, String> {
    handle
        .join()
        .map_err(|_| "removal Git helper output reader panicked".to_string())
}

fn require_success(captured: &CapturedChild, label: &str) -> Result<(), String> {
    if captured.status.success()
        && !captured.stdout.truncated
        && !captured.stderr.truncated
        && captured.stderr.bytes.is_empty()
    {
        Ok(())
    } else {
        Err(captured_error(label, captured))
    }
}

fn captured_error(label: &str, captured: &CapturedChild) -> String {
    let stderr = String::from_utf8_lossy(&captured.stderr.bytes);
    let message = stderr.trim();
    if message.is_empty() {
        format!("{label} exited with status {}", captured.status)
    } else {
        format!("{label}: {message}")
    }
}

fn one_line(bytes: &[u8], label: &str) -> Result<String, String> {
    let value = std::str::from_utf8(bytes)
        .map_err(|error| format!("{label} was not UTF-8: {error}"))?
        .trim_end_matches(['\r', '\n']);
    if value.is_empty() || value.contains(['\r', '\n']) {
        return Err(format!("{label} did not contain exactly one value"));
    }
    Ok(value.to_string())
}
