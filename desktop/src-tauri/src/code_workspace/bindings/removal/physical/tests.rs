use std::collections::BTreeMap;
use std::fs;
#[cfg(target_os = "linux")]
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileTypeExt;
#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Barrier, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::*;
use crate::code_workspace::bindings::{
    CodeExecutionMode, CodeThreadBindingLookupInput, CodeThreadBindingScope,
    CodeThreadBindingStore, CodeThreadLifecycleStatus,
};
use crate::code_workspace::worktrees::{
    prepare_execution_root_with_merge_target, CodeWorktreeDescriptor, CodeWorktreePrepareInput,
};

const PREPARATION_ID: &str = "11111111-1111-4111-8111-111111111111";
const THREAD_ID: &str = "thread-physical-removal";
const CRASH_REQUEST_ENV: &str = "SCHOOLX_CODE_REMOVAL_CRASH_REQUEST_V1";
const CRASH_REQUEST_VERSION: u32 = 1;
const MAX_CRASH_REQUEST_BYTES: usize = 64 * 1024;
const INJECTED_CRASH_EXIT_CODE: i32 = 86;

struct PhysicalFixture {
    _directory: tempfile::TempDir,
    app_data: PathBuf,
    nest_root: PathBuf,
    source_root: PathBuf,
    managed_root: PathBuf,
    sibling_root: PathBuf,
    transcript: PathBuf,
    external_symlink_target: PathBuf,
    store: CodeThreadBindingStore,
    lookup: CodeThreadBindingLookupInput,
}

#[cfg(target_os = "linux")]
const LINUX_MOUNT_MODE_ENV: &str = "SCHOOLX_CODE_REMOVAL_TEST_MOUNT_MODE";

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
enum LinuxMountMode {
    Direct,
    Sudo,
}

#[cfg(target_os = "linux")]
impl LinuxMountMode {
    fn from_env() -> Result<Self, String> {
        match std::env::var(LINUX_MOUNT_MODE_ENV) {
            Ok(value) if value == "direct" => Ok(Self::Direct),
            Ok(value) if value == "sudo" => Ok(Self::Sudo),
            Ok(value) => Err(format!(
                "{LINUX_MOUNT_MODE_ENV} must be direct or sudo, got {value:?}"
            )),
            Err(error) => Err(format!(
                "ignored Linux mount fixture requires {LINUX_MOUNT_MODE_ENV}=direct|sudo: {error}"
            )),
        }
    }
}

#[cfg(target_os = "linux")]
fn run_linux_mount_utility(
    mode: LinuxMountMode,
    utility: &str,
    arguments: &[&std::ffi::OsStr],
) -> Result<(), String> {
    let executable = crate::managed_agents::resolve_command(utility)
        .ok_or_else(|| format!("Linux removal fixture could not resolve {utility}"))?;
    let mut command = match mode {
        LinuxMountMode::Direct => Command::new(&executable),
        LinuxMountMode::Sudo => {
            let sudo = crate::managed_agents::resolve_command("sudo")
                .ok_or_else(|| "Linux removal fixture could not resolve sudo".to_string())?;
            let mut command = Command::new(sudo);
            command.arg("-n").arg(&executable);
            command
        }
    };
    let output = command
        .args(arguments)
        .output()
        .map_err(|error| format!("failed to execute Linux removal fixture {utility}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Linux removal fixture {utility} failed with {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(target_os = "linux")]
#[derive(Debug, Eq, PartialEq)]
struct LinuxNodeProbe {
    device: u64,
    inode: u64,
    birth_time: std::time::SystemTime,
    mount_id: u64,
}

#[cfg(target_os = "linux")]
impl LinuxNodeProbe {
    fn capture(path: &Path) -> Result<Self, String> {
        let metadata = fs::metadata(path)
            .map_err(|error| format!("failed to inspect Linux bind target: {error}"))?;
        let birth_time = metadata
            .created()
            .map_err(|error| format!("Linux bind target has no birth-time identity: {error}"))?;
        let file = fs::File::open(path)
            .map_err(|error| format!("failed to open Linux bind target: {error}"))?;
        let fdinfo = fs::read_to_string(format!("/proc/self/fdinfo/{}", file.as_raw_fd()))
            .map_err(|error| format!("failed to read Linux bind target mount id: {error}"))?;
        let mount_ids = fdinfo
            .lines()
            .filter_map(|line| line.strip_prefix("mnt_id:"))
            .map(str::trim)
            .map(|value| {
                value
                    .parse::<u64>()
                    .map_err(|error| format!("invalid Linux bind target mount id: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let [mount_id] = mount_ids.as_slice() else {
            return Err("Linux bind target did not expose exactly one mount id".to_string());
        };
        if *mount_id == 0 {
            return Err("Linux bind target exposed an empty mount id".to_string());
        }
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            birth_time,
            mount_id: *mount_id,
        })
    }
}

#[cfg(target_os = "linux")]
struct LinuxSelfBindMount {
    mode: LinuxMountMode,
    target: PathBuf,
    original: LinuxNodeProbe,
    mounted: bool,
}

#[cfg(target_os = "linux")]
impl LinuxSelfBindMount {
    fn install(target: &Path) -> Result<Self, String> {
        let mode = LinuxMountMode::from_env()?;
        let target = target
            .canonicalize()
            .map_err(|error| format!("failed to canonicalize Linux bind target: {error}"))?;
        let original = LinuxNodeProbe::capture(&target)?;
        run_linux_mount_utility(
            mode,
            "mount",
            &[
                std::ffi::OsStr::new("--bind"),
                target.as_os_str(),
                target.as_os_str(),
            ],
        )?;
        let mut mounted = Self {
            mode,
            target,
            original,
            mounted: true,
        };
        let current = LinuxNodeProbe::capture(&mounted.target);
        let verification = match current {
            Ok(current)
                if current.device == mounted.original.device
                    && current.inode == mounted.original.inode
                    && current.birth_time == mounted.original.birth_time
                    && current.mount_id != mounted.original.mount_id =>
            {
                Ok(())
            }
            Ok(current) => Err(format!(
                "Linux self-bind did not preserve dev/inode/birth-time while changing mount-id: before={:?}, after={current:?}",
                mounted.original
            )),
            Err(error) => Err(error),
        };
        if let Err(error) = verification {
            let cleanup = mounted.unmount().err();
            return Err(match cleanup {
                Some(cleanup) => format!("{error}; self-bind cleanup also failed: {cleanup}"),
                None => error,
            });
        }
        Ok(mounted)
    }

    fn unmount(&mut self) -> Result<(), String> {
        if !self.mounted {
            return Ok(());
        }
        run_linux_mount_utility(
            self.mode,
            "umount",
            &[std::ffi::OsStr::new("--"), self.target.as_os_str()],
        )?;
        self.mounted = false;
        let restored = LinuxNodeProbe::capture(&self.target)?;
        if restored != self.original {
            return Err(format!(
                "Linux self-bind cleanup did not restore the original identity: expected={:?}, actual={restored:?}",
                self.original
            ));
        }
        Ok(())
    }
}

#[cfg(target_os = "linux")]
impl Drop for LinuxSelfBindMount {
    fn drop(&mut self) {
        if let Err(error) = self.unmount() {
            eprintln!("failed to clean Linux removal fixture bind mount: {error}");
        }
    }
}

#[cfg(target_os = "linux")]
fn finish_linux_mounted_test(
    result: Result<(), String>,
    mount: &mut LinuxSelfBindMount,
) -> Result<(), String> {
    let cleanup = mount.unmount();
    match (result, cleanup) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(error), Err(cleanup)) => Err(format!("{error}; bind cleanup also failed: {cleanup}")),
    }
}

#[derive(Clone, Copy)]
enum TestRepositoryStorage {
    SelfContained,
    SharedClone,
}

fn test_git(cwd: &Path, arguments: &[&str]) -> Result<Vec<u8>, String> {
    let executable = crate::managed_agents::resolve_command("git")
        .ok_or_else(|| "git executable was not found".to_string())?;
    let output = Command::new(executable)
        .arg("--no-pager")
        .args(arguments)
        .current_dir(cwd)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|error| format!("failed to run test Git: {error}"))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

fn git_line(cwd: &Path, arguments: &[&str]) -> Result<String, String> {
    let stdout = test_git(cwd, arguments)?;
    let value = std::str::from_utf8(&stdout)
        .map_err(|error| format!("test Git output was not UTF-8: {error}"))?
        .trim_end_matches(['\r', '\n']);
    if value.is_empty() || value.contains(['\r', '\n']) {
        return Err("test Git output was not exactly one line".to_string());
    }
    Ok(value.to_string())
}

fn path_string(path: &Path, label: &str) -> Result<String, String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{label} was not UTF-8"))
}

fn archive(
    store: &CodeThreadBindingStore,
    lookup: &CodeThreadBindingLookupInput,
) -> Result<(), String> {
    let claim = store.begin_archive(lookup)?;
    let archived = store.complete_lifecycle_transition(&claim)?;
    if archived.status != CodeThreadLifecycleStatus::Archived {
        return Err("physical removal fixture did not reach Archived".to_string());
    }
    Ok(())
}

fn prepare_linked_worktree(
    source_root: &Path,
    nest_root: &Path,
) -> Result<(CodeWorktreeDescriptor, String), String> {
    let prepared = prepare_execution_root_with_merge_target(
        CodeWorktreePrepareInput {
            repository_root: path_string(source_root, "test repository")?,
            base_ref: "HEAD".to_string(),
            execution_mode: CodeExecutionMode::Worktree,
        },
        nest_root,
    )?;
    let target_ref = prepared
        .merge_target_ref
        .ok_or_else(|| "fixture did not capture direct local merge authority".to_string())?;
    Ok((prepared.worktree.descriptor, target_ref))
}

fn initialize_test_repository(
    repository: &Path,
    external_symlink_target: &Path,
) -> Result<(), String> {
    fs::create_dir_all(repository).map_err(|error| error.to_string())?;
    test_git(repository, &["init", "--initial-branch=main"])?;
    fs::create_dir(repository.join("nested")).map_err(|error| error.to_string())?;
    fs::write(repository.join("README.md"), b"source checkout\n")
        .map_err(|error| error.to_string())?;
    fs::write(
        repository.join("nested").join("tracked.txt"),
        b"tracked bytes\n",
    )
    .map_err(|error| error.to_string())?;
    fs::write(repository.join(".gitignore"), b"ignored-output\n")
        .map_err(|error| error.to_string())?;
    std::os::unix::fs::symlink(external_symlink_target, repository.join("README.link"))
        .map_err(|error| error.to_string())?;
    test_git(repository, &["add", "--all"])?;
    test_git(
        repository,
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
    Ok(())
}

fn prepare_fixture_with_storage(
    repository_storage: TestRepositoryStorage,
) -> Result<PhysicalFixture, String> {
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let source_root = directory.path().join("repository");
    let nest_root = directory.path().join("active-nest");
    let app_data = directory.path().join("app-data");
    let transcript = directory.path().join("transcripts").join("thread.jsonl");
    let external_symlink_target = directory.path().join("external-symlink-target.txt");
    fs::write(&external_symlink_target, b"external target bytes\n")
        .map_err(|error| error.to_string())?;
    fs::create_dir_all(&nest_root).map_err(|error| error.to_string())?;
    fs::create_dir_all(&app_data).map_err(|error| error.to_string())?;
    fs::create_dir_all(
        transcript
            .parent()
            .ok_or_else(|| "fixture transcript has no parent".to_string())?,
    )
    .map_err(|error| error.to_string())?;

    match repository_storage {
        TestRepositoryStorage::SelfContained => {
            initialize_test_repository(&source_root, &external_symlink_target)?;
        }
        TestRepositoryStorage::SharedClone => {
            let donor = directory.path().join("shared-object-donor");
            initialize_test_repository(&donor, &external_symlink_target)?;
            let donor = path_string(&donor, "shared-clone donor")?;
            let destination = path_string(&source_root, "shared-clone destination")?;
            test_git(
                directory.path(),
                &[
                    "clone",
                    "--shared",
                    "--branch",
                    "main",
                    &donor,
                    &destination,
                ],
            )?;
        }
    }

    let source_root = source_root
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize fixture repository: {error}"))?;
    let nest_root = nest_root
        .canonicalize()
        .map_err(|error| format!("failed to canonicalize fixture nest: {error}"))?;

    let (descriptor, target_ref) = prepare_linked_worktree(&source_root, &nest_root)?;
    let (sibling_descriptor, sibling_target_ref) =
        prepare_linked_worktree(&source_root, &nest_root)?;
    if sibling_target_ref != target_ref {
        return Err("fixture linked worktrees captured different target refs".to_string());
    }
    let managed_root = PathBuf::from(&descriptor.execution_root);
    let sibling_root = PathBuf::from(&sibling_descriptor.execution_root);
    fs::write(&transcript, b"preserve transcript bytes\n").map_err(|error| error.to_string())?;

    let store = CodeThreadBindingStore::for_app_data(&app_data)?;
    let scope = CodeThreadBindingScope {
        community_id: "community-physical-removal".to_string(),
        project_dtag: "project-physical-removal".to_string(),
        repository_identity: descriptor.repository_identity.clone(),
    };
    store.create_preparation_with_merge_target(
        PREPARATION_ID.to_string(),
        scope.clone(),
        &descriptor,
        Some(target_ref),
    )?;
    store.claim_preparation_for_start(&scope, PREPARATION_ID, Vec::new())?;
    let binding = store.commit_preparation_binding(&scope, PREPARATION_ID, THREAD_ID)?;
    let lookup = CodeThreadBindingLookupInput {
        scope,
        codex_thread_id: binding.codex_thread_id,
    };
    archive(&store, &lookup)?;

    Ok(PhysicalFixture {
        _directory: directory,
        app_data,
        nest_root,
        source_root,
        managed_root,
        sibling_root,
        transcript,
        external_symlink_target,
        store,
        lookup,
    })
}

fn prepare_fixture() -> Result<PhysicalFixture, String> {
    prepare_fixture_with_storage(TestRepositoryStorage::SelfContained)
}

fn snapshot_tree(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
    fn visit(
        root: &Path,
        current: &Path,
        output: &mut BTreeMap<String, Vec<u8>>,
    ) -> Result<(), String> {
        let mut entries = fs::read_dir(current)
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_string_lossy()
                .into_owned();
            let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
            if metadata.file_type().is_symlink() {
                output.insert(
                    format!("symlink:{relative}"),
                    fs::read_link(&path)
                        .map_err(|error| error.to_string())?
                        .to_string_lossy()
                        .into_owned()
                        .into_bytes(),
                );
            } else if metadata.is_dir() {
                output.insert(format!("dir:{relative}"), Vec::new());
                visit(root, &path, output)?;
            } else if metadata.is_file() {
                output.insert(
                    format!("file:{relative}"),
                    fs::read(&path).map_err(|error| error.to_string())?,
                );
            } else {
                output.insert(format!("special:{relative}"), Vec::new());
            }
        }
        Ok(())
    }

    let mut output = BTreeMap::new();
    visit(root, root, &mut output)?;
    Ok(output)
}

fn linked_admin_entry(managed_root: &Path) -> Result<PathBuf, String> {
    let contents = fs::read(managed_root.join(".git")).map_err(|error| error.to_string())?;
    let line = std::str::from_utf8(&contents)
        .map_err(|error| format!("linked-worktree .git was not UTF-8: {error}"))?
        .trim_end_matches(['\r', '\n']);
    let value = line
        .strip_prefix("gitdir: ")
        .ok_or_else(|| "linked-worktree .git did not contain gitdir authority".to_string())?;
    let path = PathBuf::from(value);
    if !path.is_absolute() || !path.is_dir() {
        return Err("linked-worktree gitdir authority was not an absolute directory".to_string());
    }
    path.canonicalize()
        .map_err(|error| format!("failed to canonicalize linked-worktree gitdir: {error}"))
}

fn assert_claim_rejected_without_mutation(
    fixture: &PhysicalFixture,
    expected_error: &str,
) -> Result<(), String> {
    let root_parent = fixture
        .managed_root
        .parent()
        .ok_or_else(|| "managed root has no parent".to_string())?;
    let admin_entry = linked_admin_entry(&fixture.managed_root)?;
    let admin_parent = admin_entry
        .parent()
        .ok_or_else(|| "Git-admin entry has no parent".to_string())?;
    let store_before = fs::read(fixture.store.store_path()).map_err(|error| error.to_string())?;
    let store_modified_before = fs::metadata(fixture.store.store_path())
        .and_then(|metadata| metadata.modified())
        .map_err(|error| error.to_string())?;
    let root_parent_before = snapshot_tree(root_parent)?;
    let admin_parent_before = snapshot_tree(admin_parent)?;
    let sibling_before = snapshot_tree(&fixture.sibling_root)?;
    let transcript_before = fs::read(&fixture.transcript).map_err(|error| error.to_string())?;
    let refs_before = test_git(
        &fixture.source_root,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname) %(symref)",
        ],
    )?;

    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut unix::NoopFaultHook,
    )
    .expect_err("adversarial fixture must reject physical-removal claim");
    assert!(
        error.contains(expected_error),
        "unexpected rejection: {error}"
    );

    assert!(fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .is_none());
    assert_eq!(
        fs::read(fixture.store.store_path()).map_err(|error| error.to_string())?,
        store_before,
        "binding store bytes changed on rejected claim"
    );
    assert_eq!(
        fs::metadata(fixture.store.store_path())
            .and_then(|metadata| metadata.modified())
            .map_err(|error| error.to_string())?,
        store_modified_before,
        "binding store mtime changed on rejected claim"
    );
    assert_eq!(snapshot_tree(root_parent)?, root_parent_before);
    assert_eq!(snapshot_tree(admin_parent)?, admin_parent_before);
    assert_eq!(snapshot_tree(&fixture.sibling_root)?, sibling_before);
    assert_eq!(
        fs::read(&fixture.transcript).map_err(|error| error.to_string())?,
        transcript_before
    );
    assert_eq!(
        test_git(
            &fixture.source_root,
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname) %(symref)"
            ],
        )?,
        refs_before,
        "Git refs changed on rejected claim"
    );
    assert!(
        !fixture
            .app_data
            .join("code")
            .join("removal-manifests-v1")
            .exists(),
        "rejected claim persisted a manifest sidecar"
    );
    Ok(())
}

fn assert_removed(
    fixture: &PhysicalFixture,
    removed: &CodeWorktreeRemovalRecord,
) -> Result<(), String> {
    if !matches!(removed, CodeWorktreeRemovalRecord::Removed(_)) {
        return Err("physical removal did not finish with a tombstone".to_string());
    }
    let authority = removed.authority();
    if fs::symlink_metadata(&authority.physical.managed_root).is_ok() {
        return Err("original managed root remained after removal".to_string());
    }
    let quarantine = Path::new(&authority.physical.managed_root_parent)
        .join(&authority.physical.quarantine_name);
    if fs::symlink_metadata(&quarantine).is_ok() {
        return Err("quarantine remained after removal".to_string());
    }
    let admin =
        Path::new(&authority.physical.git_admin_parent).join(&authority.physical.git_admin_entry);
    if fs::symlink_metadata(&admin).is_ok() {
        return Err("Git-admin entry remained after removal".to_string());
    }
    if fixture.store.lookup(&fixture.lookup)?.is_some() {
        return Err("removed binding remained executable".to_string());
    }
    let retried = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "removed tombstone was not persisted".to_string())?;
    if retried != *removed {
        return Err("removed retry did not return the exact tombstone".to_string());
    }
    let proof_ref = format!("refs/schoolx/removal-claims/{}", authority.removal_id);
    if test_git(
        &fixture.source_root,
        &["show-ref", "--verify", "--hash", &proof_ref],
    )
    .is_ok()
    {
        return Err("private removal proof ref remained after finalization".to_string());
    }
    let sidecar = fixture
        .app_data
        .join("code")
        .join("removal-manifests-v1")
        .join(format!("{}.json", authority.physical_manifest_digest));
    if fs::symlink_metadata(&sidecar).is_ok() {
        return Err("durable cleanup marker sidecar remained after success".to_string());
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
enum CrashBoundary {
    Claimed,
    Removing,
    ProofRefPinned,
    Quarantined,
    RootEntryDeleted,
    RootDeleted,
    AdminEntryDeleted,
    AdminDeleted,
    AbsenceVerified,
    Finalized,
    ProofRefCleaned,
}

impl CrashBoundary {
    const ALL: [Self; 11] = [
        Self::Claimed,
        Self::Removing,
        Self::ProofRefPinned,
        Self::Quarantined,
        Self::RootEntryDeleted,
        Self::RootDeleted,
        Self::AdminEntryDeleted,
        Self::AdminDeleted,
        Self::AbsenceVerified,
        Self::Finalized,
        Self::ProofRefCleaned,
    ];

    fn fault_boundary(self) -> unix::FaultBoundary {
        match self {
            Self::Claimed => unix::FaultBoundary::Claimed,
            Self::Removing => unix::FaultBoundary::Removing,
            Self::ProofRefPinned => unix::FaultBoundary::ProofRefPinned,
            Self::Quarantined => unix::FaultBoundary::Quarantined,
            Self::RootEntryDeleted => unix::FaultBoundary::RootEntryDeleted(0),
            Self::RootDeleted => unix::FaultBoundary::RootDeleted,
            Self::AdminEntryDeleted => unix::FaultBoundary::AdminEntryDeleted(0),
            Self::AdminDeleted => unix::FaultBoundary::AdminDeleted,
            Self::AbsenceVerified => unix::FaultBoundary::AbsenceVerified,
            Self::Finalized => unix::FaultBoundary::Finalized,
            Self::ProofRefCleaned => unix::FaultBoundary::ProofRefCleaned,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CrashRequest {
    version: u32,
    app_data: String,
    nest_root: String,
    lookup: CodeThreadBindingLookupInput,
    boundary: CrashBoundary,
}

struct ExitProcessAt {
    target: unix::FaultBoundary,
}

impl unix::FaultHook for ExitProcessAt {
    fn after(&mut self, boundary: unix::FaultBoundary) -> Result<(), String> {
        if boundary == self.target {
            std::process::exit(INJECTED_CRASH_EXIT_CODE);
        }
        Ok(())
    }
}

fn crash_request_for(
    fixture: &PhysicalFixture,
    boundary: CrashBoundary,
) -> Result<CrashRequest, String> {
    Ok(CrashRequest {
        version: CRASH_REQUEST_VERSION,
        app_data: path_string(&fixture.app_data, "crash fixture app data")?,
        nest_root: path_string(&fixture.nest_root, "crash fixture nest")?,
        lookup: fixture.lookup.clone(),
        boundary,
    })
}

fn spawn_crash_child(request: &CrashRequest) -> Result<(), String> {
    let encoded = serde_json::to_string(request)
        .map_err(|error| format!("failed to encode crash request: {error}"))?;
    if encoded.len() > MAX_CRASH_REQUEST_BYTES {
        return Err(format!(
            "crash request exceeds {MAX_CRASH_REQUEST_BYTES} bytes"
        ));
    }
    let executable = std::env::current_exe()
        .map_err(|error| format!("failed to resolve crash test executable: {error}"))?;
    let output = Command::new(executable)
        .args([
            "--exact",
            "code_workspace::bindings::removal::physical::tests::physical_removal_crash_subprocess_entry",
            "--ignored",
            "--nocapture",
        ])
        .env(CRASH_REQUEST_ENV, encoded)
        .output()
        .map_err(|error| format!("failed to start physical-removal crash child: {error}"))?;
    if output.status.code() != Some(INJECTED_CRASH_EXIT_CODE) {
        return Err(format!(
            "crash child for {:?} exited as {:?}; stdout={}; stderr={}",
            request.boundary,
            output.status,
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim(),
        ));
    }
    Ok(())
}

fn run_crash_child_from_env() -> Result<(), String> {
    let encoded = std::env::var(CRASH_REQUEST_ENV)
        .map_err(|_| "physical-removal crash request is missing".to_string())?;
    if encoded.is_empty() || encoded.len() > MAX_CRASH_REQUEST_BYTES {
        return Err(format!(
            "physical-removal crash request must be 1..={MAX_CRASH_REQUEST_BYTES} bytes"
        ));
    }
    let request: CrashRequest = serde_json::from_str(&encoded)
        .map_err(|error| format!("physical-removal crash request is invalid: {error}"))?;
    if request.version != CRASH_REQUEST_VERSION {
        return Err(format!(
            "unsupported physical-removal crash request version {}",
            request.version
        ));
    }
    let app_data = PathBuf::from(&request.app_data);
    let nest_root = PathBuf::from(&request.nest_root);
    if !app_data.is_absolute() || !app_data.is_dir() {
        return Err("physical-removal crash app data is not an absolute directory".to_string());
    }
    if !nest_root.is_absolute()
        || nest_root
            .canonicalize()
            .map_err(|error| format!("failed to resolve crash nest: {error}"))?
            != nest_root
    {
        return Err("physical-removal crash nest is not canonical".to_string());
    }
    let store = CodeThreadBindingStore::for_app_data(&app_data)?;
    let mut hook = ExitProcessAt {
        target: request.boundary.fault_boundary(),
    };
    let result = unix::claim_or_resume(
        &store,
        &request.lookup,
        &nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut hook,
    );
    Err(format!(
        "physical-removal crash child returned before {:?}: {result:?}",
        request.boundary
    ))
}

#[test]
fn clean_linked_worktree_is_physically_removed_without_mutating_siblings_or_transcript(
) -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let source_head = git_line(&fixture.source_root, &["rev-parse", "HEAD"])?;
    let source_status = test_git(&fixture.source_root, &["status", "--porcelain=v1", "-z"])?;
    let sibling_before = snapshot_tree(&fixture.sibling_root)?;
    let transcript_before = fs::read(&fixture.transcript).map_err(|error| error.to_string())?;

    let removed = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut unix::NoopFaultHook,
    )?;

    assert_removed(&fixture, &removed)?;
    assert_eq!(
        snapshot_tree(&fixture.sibling_root)?,
        sibling_before,
        "sibling worktree bytes changed"
    );
    assert_eq!(
        fs::read(&fixture.transcript).map_err(|error| error.to_string())?,
        transcript_before,
        "Codex transcript bytes changed"
    );
    assert_eq!(
        git_line(&fixture.source_root, &["rev-parse", "HEAD"])?,
        source_head,
        "authorized branch changed"
    );
    assert_eq!(
        test_git(&fixture.source_root, &["status", "--porcelain=v1", "-z"])?,
        source_status,
        "source checkout changed"
    );
    Ok(())
}

#[test]
fn tracked_external_symlink_removal_preserves_target_bytes() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let target_before = fs::read(&fixture.external_symlink_target)
        .map_err(|error| format!("failed to read external symlink target: {error}"))?;
    assert_eq!(
        fs::read_link(fixture.managed_root.join("README.link"))
            .map_err(|error| format!("failed to read tracked external symlink: {error}"))?,
        fixture.external_symlink_target
    );

    let removed = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut unix::NoopFaultHook,
    )?;

    assert_removed(&fixture, &removed)?;
    assert_eq!(
        fs::read(&fixture.external_symlink_target)
            .map_err(|error| format!("external symlink target was not preserved: {error}"))?,
        target_before
    );
    Ok(())
}

#[test]
fn shared_clone_alternates_reject_claim_with_zero_mutation() -> Result<(), String> {
    let fixture = prepare_fixture_with_storage(TestRepositoryStorage::SharedClone)?;
    let alternates = fixture
        .source_root
        .join(".git")
        .join("objects")
        .join("info")
        .join("alternates");
    let alternate_bytes = fs::read(&alternates)
        .map_err(|error| format!("git clone --shared did not create alternates: {error}"))?;
    assert!(
        !alternate_bytes.is_empty(),
        "git clone --shared created an empty alternates file"
    );

    assert_claim_rejected_without_mutation(&fixture, "alternate object storage")?;
    assert_eq!(
        fs::read(&alternates).map_err(|error| error.to_string())?,
        alternate_bytes,
        "rejected claim changed the shared-clone alternates file"
    );
    Ok(())
}

#[test]
fn git_worktree_lock_rejects_claim_with_zero_mutation() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let managed_root = path_string(&fixture.managed_root, "managed worktree")?;
    test_git(
        &fixture.source_root,
        &[
            "worktree",
            "lock",
            "--reason",
            "SchoolX adversarial removal test",
            &managed_root,
        ],
    )?;
    let admin = linked_admin_entry(&fixture.managed_root)?;
    assert!(
        admin.join("locked").is_file(),
        "git worktree lock did not create the Git-admin lock marker"
    );

    assert_claim_rejected_without_mutation(
        &fixture,
        "refuses a locked or concurrently mutated Git-admin entry",
    )
}

#[test]
fn missing_local_head_blob_rejects_claim_with_zero_mutation() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let object_id = git_line(&fixture.source_root, &["rev-parse", "HEAD:README.md"])?;
    if object_id.len() < 3 || !object_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("fixture README blob object id was invalid".to_string());
    }
    let object_path = fixture
        .source_root
        .join(".git")
        .join("objects")
        .join(&object_id[..2])
        .join(&object_id[2..]);
    let object_bytes = fs::read(&object_path).map_err(|error| {
        format!("fixture README blob was not an ordinary loose object: {error}")
    })?;
    let parked_object = fixture._directory.path().join("parked-readme-blob");
    fs::rename(&object_path, &parked_object)
        .map_err(|error| format!("failed to hide fixture README blob: {error}"))?;
    assert!(
        fs::symlink_metadata(&object_path).is_err(),
        "fixture README blob remained locally available"
    );

    assert_claim_rejected_without_mutation(&fixture, "requires every HEAD blob to exist locally")?;
    assert!(
        fs::symlink_metadata(&object_path).is_err(),
        "rejected claim recreated the missing local blob"
    );
    assert_eq!(
        fs::read(&parked_object).map_err(|error| error.to_string())?,
        object_bytes,
        "rejected claim changed the parked blob bytes"
    );
    Ok(())
}

#[test]
fn untracked_empty_directory_rejects_claim_with_zero_mutation() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    fs::create_dir(fixture.managed_root.join("untracked-empty-directory"))
        .map_err(|error| error.to_string())?;
    assert!(
        test_git(&fixture.managed_root, &["status", "--porcelain=v1", "-z"])?.is_empty(),
        "Git unexpectedly reported an empty untracked directory"
    );

    assert_claim_rejected_without_mutation(&fixture, "rejects unexpected worktree entry")
}

#[test]
fn ignored_fifo_rejects_claim_with_zero_mutation() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let fifo_path = fixture.managed_root.join("ignored-output");
    let executable = crate::managed_agents::resolve_command("mkfifo")
        .ok_or_else(|| "mkfifo executable was not found".to_string())?;
    let output = Command::new(executable)
        .arg(&fifo_path)
        .output()
        .map_err(|error| format!("failed to create adversarial FIFO: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "failed to create adversarial FIFO: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    assert!(
        test_git(&fixture.managed_root, &["status", "--porcelain=v1", "-z"])?.is_empty(),
        "fixture ignored special entry unexpectedly appeared in ordinary status"
    );
    assert!(
        fs::symlink_metadata(&fifo_path)
            .map_err(|error| error.to_string())?
            .file_type()
            .is_fifo(),
        "adversarial entry was not a FIFO"
    );

    assert_claim_rejected_without_mutation(&fixture, "rejects unexpected worktree entry")
}

#[test]
fn child_process_crash_reopen_matrix_recovers_every_durable_boundary() -> Result<(), String> {
    for boundary in CrashBoundary::ALL {
        let result = (|| -> Result<(), String> {
            let fixture = prepare_fixture()?;
            let sibling_before = snapshot_tree(&fixture.sibling_root)?;
            let transcript_before =
                fs::read(&fixture.transcript).map_err(|error| error.to_string())?;
            let source_head = git_line(&fixture.source_root, &["rev-parse", "HEAD"])?;
            let source_status =
                test_git(&fixture.source_root, &["status", "--porcelain=v1", "-z"])?;

            spawn_crash_child(&crash_request_for(&fixture, boundary)?)?;

            let interrupted_store = CodeThreadBindingStore::for_app_data(&fixture.app_data)?;
            let interrupted = interrupted_store
                .lookup_worktree_removal(&fixture.lookup)?
                .ok_or_else(|| "crash did not preserve a removal journal".to_string())?;
            match (&interrupted, boundary) {
                (CodeWorktreeRemovalRecord::Claimed(_), CrashBoundary::Claimed)
                | (
                    CodeWorktreeRemovalRecord::Removed(_),
                    CrashBoundary::Finalized | CrashBoundary::ProofRefCleaned,
                )
                | (
                    CodeWorktreeRemovalRecord::Removing(_),
                    CrashBoundary::Removing
                    | CrashBoundary::ProofRefPinned
                    | CrashBoundary::Quarantined
                    | CrashBoundary::RootEntryDeleted
                    | CrashBoundary::RootDeleted
                    | CrashBoundary::AdminEntryDeleted
                    | CrashBoundary::AdminDeleted
                    | CrashBoundary::AbsenceVerified,
                ) => {}
                _ => {
                    return Err(format!(
                        "crash at {boundary:?} persisted the wrong journal state: {interrupted:?}"
                    ))
                }
            }
            let removal_id = interrupted.authority().removal_id.clone();

            let reopened = CodeThreadBindingStore::for_app_data(&fixture.app_data)?;
            unix::recover_all(&reopened, &fixture.nest_root, &mut unix::NoopFaultHook)?;
            let removed = reopened
                .lookup_worktree_removal(&fixture.lookup)?
                .ok_or_else(|| "recovery lost the removal tombstone".to_string())?;
            if !matches!(removed, CodeWorktreeRemovalRecord::Removed(_)) {
                return Err("recovery did not finish with a removed tombstone".to_string());
            }
            if removed.authority().removal_id != removal_id {
                return Err("recovery replaced the durable removal id".to_string());
            }
            assert_removed(&fixture, &removed)?;
            assert_eq!(
                snapshot_tree(&fixture.sibling_root)?,
                sibling_before,
                "sibling worktree bytes changed after {boundary:?}"
            );
            assert_eq!(
                fs::read(&fixture.transcript).map_err(|error| error.to_string())?,
                transcript_before,
                "Codex transcript bytes changed after {boundary:?}"
            );
            assert_eq!(
                git_line(&fixture.source_root, &["rev-parse", "HEAD"])?,
                source_head,
                "authorized branch changed after {boundary:?}"
            );
            assert_eq!(
                test_git(&fixture.source_root, &["status", "--porcelain=v1", "-z"])?,
                source_status,
                "source checkout changed after {boundary:?}"
            );
            Ok(())
        })();
        result.map_err(|error| format!("crash/reopen matrix failed at {boundary:?}: {error}"))?;
    }
    Ok(())
}

struct FailOnceAt {
    target: unix::FaultBoundary,
    tripped: bool,
}

impl unix::FaultHook for FailOnceAt {
    fn after(&mut self, boundary: unix::FaultBoundary) -> Result<(), String> {
        if !self.tripped && boundary == self.target {
            self.tripped = true;
            Err(format!("injected crash after {boundary:?}"))
        } else {
            Ok(())
        }
    }
}

#[cfg(target_os = "linux")]
fn prepare_durable_removing(
    fixture: &PhysicalFixture,
) -> Result<CodeWorktreeRemovalRecord, String> {
    let mut fault = FailOnceAt {
        target: unix::FaultBoundary::Removing,
        tripped: false,
    };
    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut fault,
    )
    .expect_err("Removing boundary fault must interrupt physical removal");
    if !fault.tripped || !error.contains("injected crash after Removing") {
        return Err(format!(
            "Linux removal fixture did not stop at durable Removing: {error}"
        ));
    }
    let removing = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "Linux removal fixture lost its durable journal".to_string())?;
    if !matches!(removing, CodeWorktreeRemovalRecord::Removing(_)) {
        return Err("Linux removal fixture did not persist Removing".to_string());
    }
    Ok(removing)
}

#[cfg(target_os = "linux")]
fn assert_positive_birth_time_identities(value: &serde_json::Value) -> Result<usize, String> {
    fn visit(value: &serde_json::Value, count: &mut usize) -> Result<(), String> {
        match value {
            serde_json::Value::Array(values) => {
                for value in values {
                    visit(value, count)?;
                }
            }
            serde_json::Value::Object(object) => {
                if let Some(seconds) = object.get("birthTimeSeconds") {
                    let seconds = seconds.as_i64().ok_or_else(|| {
                        "Linux removal manifest birthTimeSeconds is not an integer".to_string()
                    })?;
                    let nanoseconds = object
                        .get("birthTimeNanoseconds")
                        .and_then(serde_json::Value::as_u64)
                        .ok_or_else(|| {
                            "Linux removal manifest omitted birthTimeNanoseconds".to_string()
                        })?;
                    if seconds <= 0 || nanoseconds >= 1_000_000_000 {
                        return Err(format!(
                            "Linux removal manifest has invalid birth-time identity {seconds}:{nanoseconds}"
                        ));
                    }
                    *count += 1;
                }
                for value in object.values() {
                    visit(value, count)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    let mut count = 0;
    visit(value, &mut count)?;
    if count < 5 {
        return Err(format!(
            "Linux removal manifest exposed only {count} birth-time identities"
        ));
    }
    Ok(count)
}

#[cfg(target_os = "linux")]
#[test]
fn linux_removing_crash_reopen_recovers_positive_birth_time_identities() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let sibling_before = snapshot_tree(&fixture.sibling_root)?;
    let transcript_before = fs::read(&fixture.transcript).map_err(|error| error.to_string())?;
    spawn_crash_child(&crash_request_for(&fixture, CrashBoundary::Removing)?)?;

    let reopened = CodeThreadBindingStore::for_app_data(&fixture.app_data)?;
    let removing = reopened
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "Linux Removing crash lost its durable journal".to_string())?;
    if !matches!(removing, CodeWorktreeRemovalRecord::Removing(_)) {
        return Err("Linux Removing crash did not reopen in Removing".to_string());
    }
    let removal_id = removing.authority().removal_id.clone();
    let sidecar = fixture
        .app_data
        .join("code")
        .join("removal-manifests-v1")
        .join(format!(
            "{}.json",
            removing.authority().physical_manifest_digest
        ));
    let sidecar_bytes = fs::read(&sidecar)
        .map_err(|error| format!("Linux recovery sidecar is missing: {error}"))?;
    let manifest: serde_json::Value = serde_json::from_slice(&sidecar_bytes)
        .map_err(|error| format!("Linux recovery sidecar is invalid: {error}"))?;
    let identity_count = assert_positive_birth_time_identities(&manifest)?;
    if identity_count < 5 {
        return Err("Linux recovery sidecar did not pin all root identities".to_string());
    }

    unix::recover_all(&reopened, &fixture.nest_root, &mut unix::NoopFaultHook)?;
    let removed = reopened
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "Linux birth-time recovery lost its tombstone".to_string())?;
    if removed.authority().removal_id != removal_id {
        return Err("Linux birth-time recovery replaced the removal id".to_string());
    }
    assert_removed(&fixture, &removed)?;
    if snapshot_tree(&fixture.sibling_root)? != sibling_before {
        return Err("Linux birth-time recovery changed the sibling worktree".to_string());
    }
    if fs::read(&fixture.transcript).map_err(|error| error.to_string())? != transcript_before {
        return Err("Linux birth-time recovery changed the transcript".to_string());
    }
    Ok(())
}

struct DeleteFutureRootEntryAtQuarantine<'a> {
    store: &'a CodeThreadBindingStore,
    lookup: &'a CodeThreadBindingLookupInput,
    tripped: bool,
}

impl unix::FaultHook for DeleteFutureRootEntryAtQuarantine<'_> {
    fn after(&mut self, boundary: unix::FaultBoundary) -> Result<(), String> {
        if self.tripped || boundary != unix::FaultBoundary::Quarantined {
            return Ok(());
        }
        let removing = self
            .store
            .lookup_worktree_removal(self.lookup)?
            .ok_or_else(|| "quarantine hook could not find the removal journal".to_string())?;
        let authority = removing.authority();
        let quarantine = Path::new(&authority.physical.managed_root_parent)
            .join(&authority.physical.quarantine_name);
        fs::remove_file(quarantine.join(".git"))
            .map_err(|error| format!("failed to delete future manifest entry: {error}"))?;
        self.tripped = true;
        Err("injected non-prefix manifest deletion after Quarantined".to_string())
    }
}

struct InstallOriginalReplacementAtQuarantine {
    original: PathBuf,
    sentinel: Vec<u8>,
    tripped: bool,
}

impl unix::FaultHook for InstallOriginalReplacementAtQuarantine {
    fn after(&mut self, boundary: unix::FaultBoundary) -> Result<(), String> {
        if self.tripped || boundary != unix::FaultBoundary::Quarantined {
            return Ok(());
        }
        fs::create_dir(&self.original)
            .map_err(|error| format!("failed to install original-path replacement: {error}"))?;
        fs::write(self.original.join("sentinel.txt"), &self.sentinel)
            .map_err(|error| format!("failed to write replacement sentinel: {error}"))?;
        self.tripped = true;
        Err("injected original-path replacement after Quarantined".to_string())
    }
}

struct ReplaceGitAdminAtRootDeleted<'a> {
    store: &'a CodeThreadBindingStore,
    lookup: &'a CodeThreadBindingLookupInput,
    relocated_admin: PathBuf,
    sentinel: Vec<u8>,
    tripped: bool,
}

impl unix::FaultHook for ReplaceGitAdminAtRootDeleted<'_> {
    fn after(&mut self, boundary: unix::FaultBoundary) -> Result<(), String> {
        if self.tripped || boundary != unix::FaultBoundary::RootDeleted {
            return Ok(());
        }
        let removing = self
            .store
            .lookup_worktree_removal(self.lookup)?
            .ok_or_else(|| "root-deleted hook could not find the removal journal".to_string())?;
        let authority = removing.authority();
        let admin = Path::new(&authority.physical.git_admin_parent)
            .join(&authority.physical.git_admin_entry);
        fs::rename(&admin, &self.relocated_admin)
            .map_err(|error| format!("failed to relocate exact Git-admin entry: {error}"))?;
        fs::create_dir(&admin)
            .map_err(|error| format!("failed to install Git-admin replacement: {error}"))?;
        fs::write(admin.join("sentinel.txt"), &self.sentinel)
            .map_err(|error| format!("failed to write Git-admin replacement sentinel: {error}"))?;
        self.tripped = true;
        Err("injected Git-admin replacement after RootDeleted".to_string())
    }
}

struct ReplaceQuarantineWithSiblingAtQuarantined<'a> {
    store: &'a CodeThreadBindingStore,
    lookup: &'a CodeThreadBindingLookupInput,
    relocated_name: String,
    sentinel: Vec<u8>,
    tripped: bool,
}

impl unix::FaultHook for ReplaceQuarantineWithSiblingAtQuarantined<'_> {
    fn after(&mut self, boundary: unix::FaultBoundary) -> Result<(), String> {
        if self.tripped || boundary != unix::FaultBoundary::Quarantined {
            return Ok(());
        }
        let removing = self
            .store
            .lookup_worktree_removal(self.lookup)?
            .ok_or_else(|| "quarantine replacement hook lost the removal journal".to_string())?;
        let authority = removing.authority();
        let parent = Path::new(&authority.physical.managed_root_parent);
        let quarantine = parent.join(&authority.physical.quarantine_name);
        let relocated = parent.join(&self.relocated_name);
        fs::rename(&quarantine, &relocated)
            .map_err(|error| format!("failed to relocate exact quarantine: {error}"))?;
        fs::create_dir(&quarantine)
            .map_err(|error| format!("failed to install quarantine replacement: {error}"))?;
        fs::write(quarantine.join("sentinel.txt"), &self.sentinel)
            .map_err(|error| format!("failed to write quarantine replacement sentinel: {error}"))?;
        self.tripped = true;
        Err("injected quarantine sibling replacement after Quarantined".to_string())
    }
}

struct ReplaceSidecarAtFinalized<'a> {
    store: &'a CodeThreadBindingStore,
    lookup: &'a CodeThreadBindingLookupInput,
    relocated_sidecar: PathBuf,
    sentinel: Vec<u8>,
    tripped: bool,
}

impl unix::FaultHook for ReplaceSidecarAtFinalized<'_> {
    fn after(&mut self, boundary: unix::FaultBoundary) -> Result<(), String> {
        if self.tripped || boundary != unix::FaultBoundary::Finalized {
            return Ok(());
        }
        let removed = self
            .store
            .lookup_worktree_removal(self.lookup)?
            .ok_or_else(|| "finalized sidecar hook lost the removal tombstone".to_string())?;
        let authority = removed.authority();
        let sidecar = self
            .store
            .code_dir
            .join("removal-manifests-v1")
            .join(format!("{}.json", authority.physical_manifest_digest));
        fs::rename(&sidecar, &self.relocated_sidecar)
            .map_err(|error| format!("failed to relocate exact manifest sidecar: {error}"))?;
        fs::write(&sidecar, &self.sentinel)
            .map_err(|error| format!("failed to install manifest sidecar replacement: {error}"))?;
        self.tripped = true;
        Ok(())
    }
}

#[derive(Debug, Eq, PartialEq)]
struct RemovingRetrySnapshot {
    store_bytes: Vec<u8>,
    store_modified: std::time::SystemTime,
    root_parent: BTreeMap<String, Vec<u8>>,
    admin_parent: BTreeMap<String, Vec<u8>>,
    manifest_sidecars: BTreeMap<String, Vec<u8>>,
    sibling: BTreeMap<String, Vec<u8>>,
    transcript: Vec<u8>,
    refs: Vec<u8>,
}

fn snapshot_removing_retry_state(
    fixture: &PhysicalFixture,
    removing: &CodeWorktreeRemovalRecord,
) -> Result<RemovingRetrySnapshot, String> {
    let authority = removing.authority();
    let manifest_directory = fixture.app_data.join("code").join("removal-manifests-v1");
    Ok(RemovingRetrySnapshot {
        store_bytes: fs::read(fixture.store.store_path()).map_err(|error| error.to_string())?,
        store_modified: fs::metadata(fixture.store.store_path())
            .and_then(|metadata| metadata.modified())
            .map_err(|error| error.to_string())?,
        root_parent: snapshot_tree(Path::new(&authority.physical.managed_root_parent))?,
        admin_parent: snapshot_tree(Path::new(&authority.physical.git_admin_parent))?,
        manifest_sidecars: snapshot_tree(&manifest_directory)?,
        sibling: snapshot_tree(&fixture.sibling_root)?,
        transcript: fs::read(&fixture.transcript).map_err(|error| error.to_string())?,
        refs: test_git(
            &fixture.source_root,
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname) %(symref)",
            ],
        )?,
    })
}

#[cfg(target_os = "linux")]
fn assert_linux_claim_self_bind_rejected(
    fixture: &PhysicalFixture,
    target: &Path,
    expected_error: &str,
) -> Result<(), String> {
    let mut mount = LinuxSelfBindMount::install(target)?;
    let result = assert_claim_rejected_without_mutation(fixture, expected_error);
    finish_linux_mounted_test(result, &mut mount)
}

#[cfg(target_os = "linux")]
fn assert_linux_removing_self_bind_rejected(
    fixture: &PhysicalFixture,
    removing: &CodeWorktreeRemovalRecord,
    target: &Path,
    expected_error: &str,
) -> Result<(), String> {
    let record_before = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "Linux self-bind retry lost its Removing record".to_string())?;
    if record_before != *removing {
        return Err("Linux self-bind retry started from a different record".to_string());
    }
    let store_before = fs::read(fixture.store.store_path()).map_err(|error| error.to_string())?;
    let mut mount = LinuxSelfBindMount::install(target)?;
    let before = snapshot_removing_retry_state(fixture, removing)?;
    let result = (|| {
        let error = unix::claim_or_resume(
            &fixture.store,
            &fixture.lookup,
            &fixture.nest_root,
            Instant::now() + Duration::from_secs(120),
            &mut unix::NoopFaultHook,
        )
        .expect_err("Linux self-bind replacement must reject Removing retry");
        if !error.contains(expected_error) {
            return Err(format!(
                "unexpected Linux self-bind retry rejection: {error}"
            ));
        }
        if fixture.store.lookup_worktree_removal(&fixture.lookup)? != Some(removing.clone()) {
            return Err("Linux self-bind retry changed the sticky Removing record".to_string());
        }
        if fs::read(fixture.store.store_path()).map_err(|error| error.to_string())? != store_before
        {
            return Err("Linux self-bind retry changed the binding-store bytes".to_string());
        }
        if snapshot_removing_retry_state(fixture, removing)? != before {
            return Err("Linux self-bind retry mutated protected state".to_string());
        }
        Ok(())
    })();
    finish_linux_mounted_test(result, &mut mount)
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Linux mount authority; set SCHOOLX_CODE_REMOVAL_TEST_MOUNT_MODE"]
fn linux_privileged_same_filesystem_bind_managed_root_rejects_claim() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    assert_linux_claim_self_bind_rejected(
        &fixture,
        &fixture.managed_root,
        "managed worktree root crosses a nested mount boundary",
    )
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Linux mount authority; set SCHOOLX_CODE_REMOVAL_TEST_MOUNT_MODE"]
fn linux_privileged_same_filesystem_bind_tracked_entry_rejects_claim() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    assert_linux_claim_self_bind_rejected(
        &fixture,
        &fixture.managed_root.join("README.md"),
        "rejects a nested-mount manifest file",
    )
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Linux mount authority; set SCHOOLX_CODE_REMOVAL_TEST_MOUNT_MODE"]
fn linux_privileged_same_filesystem_bind_git_admin_entry_rejects_claim() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let admin = linked_admin_entry(&fixture.managed_root)?;
    assert_linux_claim_self_bind_rejected(
        &fixture,
        &admin,
        "Git-admin entry crosses a nested mount boundary",
    )
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Linux mount authority; set SCHOOLX_CODE_REMOVAL_TEST_MOUNT_MODE"]
fn linux_privileged_same_filesystem_bind_primary_objects_rejects_claim() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let objects = fixture.source_root.join(".git").join("objects");
    assert_linux_claim_self_bind_rejected(
        &fixture,
        &objects,
        "Git primary object directory crosses a nested mount boundary",
    )
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Linux mount authority; set SCHOOLX_CODE_REMOVAL_TEST_MOUNT_MODE"]
fn linux_privileged_same_filesystem_bind_sidecar_directory_preserves_removing() -> Result<(), String>
{
    let fixture = prepare_fixture()?;
    let removing = prepare_durable_removing(&fixture)?;
    let directory = fixture.app_data.join("code").join("removal-manifests-v1");
    assert_linux_removing_self_bind_rejected(
        &fixture,
        &removing,
        &directory,
        "removal manifest directory crosses a nested mount boundary",
    )
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Linux mount authority; set SCHOOLX_CODE_REMOVAL_TEST_MOUNT_MODE"]
fn linux_privileged_same_filesystem_bind_sidecar_file_preserves_removing() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let removing = prepare_durable_removing(&fixture)?;
    let sidecar = fixture
        .app_data
        .join("code")
        .join("removal-manifests-v1")
        .join(format!(
            "{}.json",
            removing.authority().physical_manifest_digest
        ));
    assert_linux_removing_self_bind_rejected(
        &fixture,
        &removing,
        &sidecar,
        "removal manifest sidecar crosses a mount boundary",
    )
}

#[cfg(target_os = "linux")]
#[test]
#[ignore = "requires Linux mount authority; set SCHOOLX_CODE_REMOVAL_TEST_MOUNT_MODE"]
fn linux_privileged_same_filesystem_bind_managed_root_is_sticky_removing() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let removing = prepare_durable_removing(&fixture)?;
    assert_linux_removing_self_bind_rejected(
        &fixture,
        &removing,
        &fixture.managed_root,
        "replacement or ambiguous state; recovery is sticky",
    )
}

#[test]
fn startup_recovery_precedes_runtime_start_and_lifecycle_reconciliation() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let sentinel = b"startup recovery replacement bytes\n".to_vec();
    let mut hook = InstallOriginalReplacementAtQuarantine {
        original: fixture.managed_root.clone(),
        sentinel: sentinel.clone(),
        tripped: false,
    };
    let injected = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut hook,
    )
    .expect_err("startup fixture must stop after installing a replacement");
    assert!(injected.contains("injected original-path replacement"));
    assert!(hook.tripped);

    let removing = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "startup fixture lost its removal journal".to_string())?;
    if !matches!(removing, CodeWorktreeRemovalRecord::Removing(_)) {
        return Err("startup fixture did not leave sticky Removing".to_string());
    }
    let before = snapshot_removing_retry_state(&fixture, &removing)?;
    let fake = crate::commands::stateful_fake_codex(
        &path_string(&fixture.managed_root, "replacement managed root")?,
        &crate::code_workspace::code_thread_source(PREPARATION_ID)?,
        THREAD_ID,
        false,
    )?;
    let started_marker = fake.started_marker();
    let runtime = crate::code_workspace::CodeRuntime::with_executable(fake.executable.clone());
    let binding_lock = Mutex::new(());
    let lifecycle_authority = AtomicBool::new(true);

    let error = crate::commands::start_code_runtime_after_removal_recovery(
        &fixture.app_data,
        &fixture.nest_root,
        &runtime,
        &binding_lock,
        &lifecycle_authority,
        |_| Arc::new(|_| {}),
    )
    .expect_err("sticky startup recovery must fail before Codex starts");
    assert!(
        error.contains("coordinates contain a replacement or ambiguous state"),
        "unexpected startup recovery rejection: {error}"
    );
    assert!(
        !started_marker.exists(),
        "Codex started before pending physical recovery completed"
    );
    assert!(
        !lifecycle_authority.load(Ordering::Acquire),
        "startup recovery failure left lifecycle authority enabled"
    );
    assert_eq!(
        fixture.store.lookup_worktree_removal(&fixture.lookup)?,
        Some(removing.clone()),
        "startup recovery changed the sticky Removing journal"
    );
    assert_eq!(
        snapshot_removing_retry_state(&fixture, &removing)?,
        before,
        "rejected startup recovery mutated protected state"
    );
    assert_eq!(
        fs::read(fixture.managed_root.join("sentinel.txt")).map_err(|error| error.to_string())?,
        sentinel,
        "startup recovery changed the replacement root"
    );
    Ok(())
}

#[test]
fn ready_runtime_start_skips_startup_only_removal_recovery() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let sentinel = b"ready runtime replacement bytes\n".to_vec();
    let mut hook = InstallOriginalReplacementAtQuarantine {
        original: fixture.managed_root.clone(),
        sentinel,
        tripped: false,
    };
    unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut hook,
    )
    .expect_err("ready-runtime fixture must stop after installing a replacement");
    if !hook.tripped {
        return Err("ready-runtime replacement hook did not run".to_string());
    }
    let removing = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "ready-runtime fixture lost its removal journal".to_string())?;
    let before = snapshot_removing_retry_state(&fixture, &removing)?;
    let fake = crate::commands::stateful_fake_codex(
        &path_string(&fixture.managed_root, "replacement managed root")?,
        &crate::code_workspace::code_thread_source(PREPARATION_ID)?,
        THREAD_ID,
        false,
    )?;
    let runtime = crate::code_workspace::CodeRuntime::with_executable(fake.executable.clone());
    let initial = runtime.start(Arc::new(|_| {}))?;
    let binding_lock = Mutex::new(());
    let lifecycle_authority = AtomicBool::new(true);

    let repeated = crate::commands::start_code_runtime_after_removal_recovery(
        &fixture.app_data,
        &fixture.nest_root,
        &runtime,
        &binding_lock,
        &lifecycle_authority,
        |_| Arc::new(|_| {}),
    )?;
    assert_eq!(repeated.generation, initial.generation);
    assert!(lifecycle_authority.load(Ordering::Acquire));
    assert_eq!(
        fixture.store.lookup_worktree_removal(&fixture.lookup)?,
        Some(removing.clone())
    );
    assert_eq!(snapshot_removing_retry_state(&fixture, &removing)?, before);
    runtime.stop()?;
    Ok(())
}

#[test]
fn public_scope_thread_removal_returns_only_the_native_derived_receipt() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let fake = crate::commands::stateful_fake_codex(
        &path_string(&fixture.managed_root, "managed root")?,
        &crate::code_workspace::code_thread_source(PREPARATION_ID)?,
        THREAD_ID,
        false,
    )?;
    let runtime = crate::code_workspace::CodeRuntime::with_executable(fake.executable.clone());
    runtime.start(Arc::new(|_| {}))?;
    let terminals = crate::code_workspace::CodeTerminalManager::new();
    let binding_lock = Mutex::new(());
    let lifecycle_authority = AtomicBool::new(true);
    let receipt = super::super::remove_archived_worktree(
        &fixture.store,
        binding_lock
            .lock()
            .map_err(|_| "test binding lock is unavailable".to_string())?,
        super::super::CodeWorktreeRemoveInput {
            scope: fixture.lookup.scope.clone(),
            thread_id: THREAD_ID.to_string(),
        },
        &fixture.nest_root,
        super::super::CodeWorktreeRemovalContext {
            runtime: &runtime,
            terminals: &terminals,
            lifecycle_authority_ready: &lifecycle_authority,
            shutdown_started: &AtomicBool::new(false),
        },
    )?;
    let removed = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "public removal did not persist its tombstone".to_string())?;
    assert_removed(&fixture, &removed)?;
    assert_eq!(receipt.removal_id, removed.authority().removal_id);
    assert_eq!(receipt.scope, fixture.lookup.scope);
    assert_eq!(receipt.thread_id, THREAD_ID);
    assert_eq!(
        receipt.worktree_id,
        removed.authority().merge_proof.worktree_id
    );
    assert_eq!(
        receipt.head_commit,
        removed.authority().merge_proof.head_commit
    );
    assert_eq!(
        receipt.merged_into_ref,
        removed.authority().merge_proof.target_ref
    );
    assert_eq!(
        receipt.merged_into_commit,
        removed.authority().merge_proof.target_commit
    );
    assert_eq!(
        receipt.transcript_disposition,
        super::super::CodeWorktreeTranscriptDisposition::Preserved
    );
    assert_eq!(
        receipt.execution_disposition,
        super::super::CodeWorktreeExecutionDisposition::Removed
    );
    runtime.stop()?;
    Ok(())
}

#[test]
fn sealed_idle_no_pty_no_approval_clearance_serializes_concurrent_admission() -> Result<(), String>
{
    let fixture = prepare_fixture()?;
    let fake = crate::commands::stateful_fake_codex(
        &path_string(&fixture.managed_root, "managed root")?,
        &crate::code_workspace::code_thread_source(PREPARATION_ID)?,
        THREAD_ID,
        false,
    )?;
    let runtime = Arc::new(crate::code_workspace::CodeRuntime::with_executable(
        fake.executable.clone(),
    ));
    let generation = runtime.start(Arc::new(|_| {}))?.generation;
    let terminals = Arc::new(crate::code_workspace::CodeTerminalManager::new());
    let binding_lock = Arc::new(Mutex::new(()));
    let lifecycle_authority = Arc::new(AtomicBool::new(true));
    let binding_guard = binding_lock
        .lock()
        .map_err(|_| "test binding lock is unavailable".to_string())?;
    let clearance = prove_removal_activity_clearance(
        &runtime,
        &terminals,
        binding_guard,
        fixture.lookup.clone(),
    )?;
    let requests_before = fake.recorded_requests()?;
    let start = Arc::new(Barrier::new(4));

    let (turn_tx, turn_rx) = mpsc::sync_channel(1);
    let turn_thread = {
        let runtime = Arc::clone(&runtime);
        let start = Arc::clone(&start);
        let input = crate::code_workspace::CodeTurnStartInput {
            scope: fixture.lookup.scope.clone(),
            thread_id: THREAD_ID.to_string(),
            prompt: "sealed removal admission".to_string(),
            model: None,
            effort: None,
        };
        let managed_root = path_string(&fixture.managed_root, "managed root")?;
        std::thread::spawn(move || {
            start.wait();
            let _ = turn_tx.send(runtime.turn_start_at(input, &managed_root));
        })
    };

    let (approval_tx, approval_rx) = mpsc::sync_channel(1);
    let approval_thread = {
        let runtime = Arc::clone(&runtime);
        let start = Arc::clone(&start);
        std::thread::spawn(move || {
            start.wait();
            let _ = approval_tx.send(runtime.insert_pending_approval_for_test(
                generation,
                "sealed-removal-approval",
                THREAD_ID,
            ));
        })
    };

    let (terminal_tx, terminal_rx) = mpsc::sync_channel(1);
    let terminal_thread = {
        let runtime = Arc::clone(&runtime);
        let terminals = Arc::clone(&terminals);
        let binding_lock = Arc::clone(&binding_lock);
        let lifecycle_authority = Arc::clone(&lifecycle_authority);
        let start = Arc::clone(&start);
        let app_data = fixture.app_data.clone();
        let nest_root = fixture.nest_root.clone();
        let scope = fixture.lookup.scope.clone();
        std::thread::spawn(move || {
            start.wait();
            let result = crate::commands::open_terminal_for_test(
                crate::code_workspace::CodeTerminalOpenInput {
                    scope,
                    thread_id: THREAD_ID.to_string(),
                    cols: 80,
                    rows: 24,
                },
                tauri::ipc::Channel::new(|_| Ok(())),
                &app_data,
                &nest_root,
                (&runtime, &terminals, &binding_lock, &lifecycle_authority),
            );
            let _ = terminal_tx.send(result);
        })
    };

    start.wait();
    let blocked = Duration::from_millis(100);
    assert!(matches!(
        turn_rx.recv_timeout(blocked),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(matches!(
        approval_rx.recv_timeout(blocked),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert!(matches!(
        terminal_rx.recv_timeout(blocked),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert_eq!(
        fake.recorded_requests()?,
        requests_before,
        "turn bytes were admitted while removal clearance was held"
    );
    terminals.ensure_owner_absent(&fixture.lookup.scope, THREAD_ID)?;

    let removed = remove_archived_worktree_private(
        &fixture.store,
        clearance,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
    )?;
    if !matches!(removed, CodeWorktreeRemovalRecord::Removed(_)) {
        return Err("sealed removal did not finalize its tombstone".to_string());
    }
    assert_removed(&fixture, &removed)?;
    turn_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|error| format!("turn admission did not unblock: {error}"))??;
    approval_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|error| format!("approval admission did not unblock: {error}"))??;
    let terminal_error = terminal_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|error| format!("terminal admission did not unblock: {error}"))?
        .expect_err("a removed tombstone must not open a terminal after clearance");
    assert!(
        terminal_error.contains("is not bound to the requested SchoolX community"),
        "unexpected removed-terminal rejection: {terminal_error}"
    );
    turn_thread
        .join()
        .map_err(|_| "turn admission thread panicked".to_string())?;
    approval_thread
        .join()
        .map_err(|_| "approval admission thread panicked".to_string())?;
    terminal_thread
        .join()
        .map_err(|_| "terminal admission thread panicked".to_string())?;
    assert!(runtime.has_pending_approval(THREAD_ID)?);
    terminals.ensure_owner_absent(&fixture.lookup.scope, THREAD_ID)?;
    runtime.stop()?;
    terminals.shutdown()?;
    Ok(())
}

#[test]
fn crash_after_quarantine_recovers_same_removal_and_preserves_external_state() -> Result<(), String>
{
    let fixture = prepare_fixture()?;
    let sibling_before = snapshot_tree(&fixture.sibling_root)?;
    let transcript_before = fs::read(&fixture.transcript).map_err(|error| error.to_string())?;
    let mut fault = FailOnceAt {
        target: unix::FaultBoundary::Quarantined,
        tripped: false,
    };

    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut fault,
    )
    .expect_err("quarantine boundary fault should interrupt removal");
    assert!(error.contains("injected crash after Quarantined"));
    assert!(fault.tripped);

    let removing = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "crash did not preserve a removal journal".to_string())?;
    if !matches!(removing, CodeWorktreeRemovalRecord::Removing(_)) {
        return Err("crash rolled the sticky removal state back".to_string());
    }
    let removal_id = removing.authority().removal_id.clone();
    let quarantine = Path::new(&removing.authority().physical.managed_root_parent)
        .join(&removing.authority().physical.quarantine_name);
    assert!(fs::symlink_metadata(&fixture.managed_root).is_err());
    assert!(fs::symlink_metadata(&quarantine).is_ok());
    assert!(Path::new(&removing.authority().physical.git_admin_parent)
        .join(&removing.authority().physical.git_admin_entry)
        .is_dir());
    let proof_ref = format!("refs/schoolx/removal-claims/{removal_id}");
    assert_eq!(
        git_line(
            &fixture.source_root,
            &["show-ref", "--verify", "--hash", &proof_ref],
        )?,
        removing.authority().merge_proof.target_commit
    );

    let reopened = CodeThreadBindingStore::for_app_data(&fixture.app_data)?;
    unix::recover_all(&reopened, &fixture.nest_root, &mut unix::NoopFaultHook)?;
    let removed = reopened
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "recovery lost the removal tombstone".to_string())?;
    assert_eq!(removed.authority().removal_id, removal_id);
    assert_removed(&fixture, &removed)?;
    assert_eq!(snapshot_tree(&fixture.sibling_root)?, sibling_before);
    assert_eq!(
        fs::read(&fixture.transcript).map_err(|error| error.to_string())?,
        transcript_before
    );
    Ok(())
}

#[test]
fn quarantined_non_prefix_external_deletion_is_sticky_without_additional_mutation(
) -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let mut hook = DeleteFutureRootEntryAtQuarantine {
        store: &fixture.store,
        lookup: &fixture.lookup,
        tripped: false,
    };
    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut hook,
    )
    .expect_err("quarantine hook should interrupt after its external deletion");
    assert!(error.contains("injected non-prefix manifest deletion"));
    assert!(hook.tripped);

    let removing = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "external deletion lost the removal journal".to_string())?;
    if !matches!(removing, CodeWorktreeRemovalRecord::Removing(_)) {
        return Err("external deletion did not leave a sticky Removing record".to_string());
    }
    let authority = removing.authority();
    let quarantine = Path::new(&authority.physical.managed_root_parent)
        .join(&authority.physical.quarantine_name);
    assert!(quarantine.is_dir());
    assert!(
        fs::symlink_metadata(quarantine.join(".git")).is_err(),
        "future manifest entry was not externally deleted"
    );
    assert!(
        quarantine.join("nested").join("tracked.txt").is_file(),
        "the deterministic first manifest entry was unexpectedly absent"
    );
    let before = snapshot_removing_retry_state(&fixture, &removing)?;

    let retry_error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut unix::NoopFaultHook,
    )
    .expect_err("non-prefix manifest deletion must reject sticky retry");
    assert!(
        retry_error.contains("deletion outside the known prefix"),
        "unexpected retry rejection: {retry_error}"
    );
    assert_eq!(
        fixture.store.lookup_worktree_removal(&fixture.lookup)?,
        Some(removing.clone()),
        "retry changed the sticky Removing journal"
    );
    assert_eq!(
        snapshot_removing_retry_state(&fixture, &removing)?,
        before,
        "rejected retry performed an additional mutation"
    );
    Ok(())
}

#[test]
fn quarantined_original_path_replacement_is_preserved_without_tombstone() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let sentinel = b"valuable replacement bytes\n".to_vec();
    let mut hook = InstallOriginalReplacementAtQuarantine {
        original: fixture.managed_root.clone(),
        sentinel: sentinel.clone(),
        tripped: false,
    };
    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut hook,
    )
    .expect_err("quarantine hook should interrupt after installing a replacement");
    assert!(error.contains("injected original-path replacement"));
    assert!(hook.tripped);

    let removing = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "replacement injection lost the removal journal".to_string())?;
    if !matches!(removing, CodeWorktreeRemovalRecord::Removing(_)) {
        return Err("replacement injection did not leave a sticky Removing record".to_string());
    }
    let authority = removing.authority();
    let quarantine = Path::new(&authority.physical.managed_root_parent)
        .join(&authority.physical.quarantine_name);
    assert!(quarantine.is_dir());
    assert_eq!(
        fs::read(fixture.managed_root.join("sentinel.txt")).map_err(|error| error.to_string())?,
        sentinel
    );
    let before = snapshot_removing_retry_state(&fixture, &removing)?;

    let retry_error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut unix::NoopFaultHook,
    )
    .expect_err("original-coordinate replacement must reject sticky retry");
    assert!(
        retry_error.contains("coordinates contain a replacement or ambiguous state"),
        "unexpected retry rejection: {retry_error}"
    );
    assert_eq!(
        fs::read(fixture.managed_root.join("sentinel.txt")).map_err(|error| error.to_string())?,
        sentinel,
        "retry changed the replacement sentinel"
    );
    assert_eq!(
        fixture.store.lookup_worktree_removal(&fixture.lookup)?,
        Some(removing.clone()),
        "retry created a Removed tombstone"
    );
    assert_eq!(
        snapshot_removing_retry_state(&fixture, &removing)?,
        before,
        "rejected replacement retry performed an additional mutation"
    );
    Ok(())
}

#[test]
fn quarantined_sibling_move_and_replacement_are_preserved_sticky() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let relocated_name = "relocated-exact-quarantine".to_string();
    let sentinel = b"valuable quarantine replacement bytes\n".to_vec();
    let mut hook = ReplaceQuarantineWithSiblingAtQuarantined {
        store: &fixture.store,
        lookup: &fixture.lookup,
        relocated_name: relocated_name.clone(),
        sentinel: sentinel.clone(),
        tripped: false,
    };
    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut hook,
    )
    .expect_err("quarantine hook should interrupt after installing its replacement");
    assert!(error.contains("injected quarantine sibling replacement"));
    assert!(hook.tripped);

    let removing = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "quarantine replacement lost the removal journal".to_string())?;
    if !matches!(removing, CodeWorktreeRemovalRecord::Removing(_)) {
        return Err("quarantine replacement did not leave a sticky Removing record".to_string());
    }
    let authority = removing.authority();
    let root_parent = Path::new(&authority.physical.managed_root_parent);
    let relocated = root_parent.join(&relocated_name);
    let replacement = root_parent.join(&authority.physical.quarantine_name);
    assert!(relocated.join(".git").is_file());
    assert!(relocated.join("nested").join("tracked.txt").is_file());
    assert_eq!(
        fs::read(replacement.join("sentinel.txt")).map_err(|error| error.to_string())?,
        sentinel
    );
    let before = snapshot_removing_retry_state(&fixture, &removing)?;

    let retry_error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut unix::NoopFaultHook,
    )
    .expect_err("quarantine-coordinate replacement must reject sticky retry");
    assert!(
        !retry_error.is_empty(),
        "quarantine replacement retry returned an empty error"
    );
    assert!(relocated.join(".git").is_file());
    assert_eq!(
        fs::read(replacement.join("sentinel.txt")).map_err(|error| error.to_string())?,
        sentinel,
        "retry changed the quarantine replacement"
    );
    assert_eq!(
        fixture.store.lookup_worktree_removal(&fixture.lookup)?,
        Some(removing.clone()),
        "retry changed the sticky Removing journal"
    );
    assert_eq!(
        snapshot_removing_retry_state(&fixture, &removing)?,
        before,
        "retry mutated the moved quarantine or its replacement"
    );
    Ok(())
}

#[test]
fn root_deleted_git_admin_move_and_replacement_remain_cleanup_pending() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let relocated_admin = fixture._directory.path().join("relocated-exact-git-admin");
    let sentinel = b"valuable Git-admin replacement bytes\n".to_vec();
    let mut hook = ReplaceGitAdminAtRootDeleted {
        store: &fixture.store,
        lookup: &fixture.lookup,
        relocated_admin: relocated_admin.clone(),
        sentinel: sentinel.clone(),
        tripped: false,
    };
    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut hook,
    )
    .expect_err("RootDeleted hook should interrupt after replacing Git-admin authority");
    assert!(error.contains("injected Git-admin replacement after RootDeleted"));
    assert!(hook.tripped);

    let removing = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "Git-admin replacement lost the removal journal".to_string())?;
    if !matches!(removing, CodeWorktreeRemovalRecord::Removing(_)) {
        return Err("Git-admin replacement did not leave a sticky Removing record".to_string());
    }
    let authority = removing.authority();
    let quarantine = Path::new(&authority.physical.managed_root_parent)
        .join(&authority.physical.quarantine_name);
    assert!(fs::symlink_metadata(&fixture.managed_root).is_err());
    assert!(fs::symlink_metadata(&quarantine).is_err());
    assert!(relocated_admin.join("HEAD").is_file());
    assert!(relocated_admin.join("index").is_file());
    let replacement =
        Path::new(&authority.physical.git_admin_parent).join(&authority.physical.git_admin_entry);
    assert_eq!(
        fs::read(replacement.join("sentinel.txt")).map_err(|error| error.to_string())?,
        sentinel
    );
    let sidecar = fixture
        .app_data
        .join("code")
        .join("removal-manifests-v1")
        .join(format!("{}.json", authority.physical_manifest_digest));
    let sidecar_before =
        fs::read(&sidecar).map_err(|error| format!("cleanup sidecar was missing: {error}"))?;
    let proof_ref = format!("refs/schoolx/removal-claims/{}", authority.removal_id);
    assert_eq!(
        git_line(
            &fixture.source_root,
            &["show-ref", "--verify", "--hash", &proof_ref],
        )?,
        authority.merge_proof.target_commit
    );
    let relocated_before = snapshot_tree(&relocated_admin)?;
    let before = snapshot_removing_retry_state(&fixture, &removing)?;

    let retry_error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut unix::NoopFaultHook,
    )
    .expect_err("Git-admin coordinate replacement must reject sticky retry");
    assert!(
        retry_error.contains("coordinates contain a replacement or ambiguous state"),
        "unexpected retry rejection: {retry_error}"
    );
    assert_eq!(snapshot_tree(&relocated_admin)?, relocated_before);
    assert_eq!(
        fs::read(replacement.join("sentinel.txt")).map_err(|error| error.to_string())?,
        sentinel,
        "retry changed the Git-admin replacement"
    );
    assert_eq!(
        fs::read(&sidecar).map_err(|error| error.to_string())?,
        sidecar_before,
        "retry cleaned or changed the removal sidecar"
    );
    assert_eq!(
        git_line(
            &fixture.source_root,
            &["show-ref", "--verify", "--hash", &proof_ref],
        )?,
        authority.merge_proof.target_commit,
        "retry cleaned or changed the removal proof ref"
    );
    assert_eq!(
        fixture.store.lookup_worktree_removal(&fixture.lookup)?,
        Some(removing.clone()),
        "retry changed the sticky Removing journal"
    );
    assert_eq!(
        snapshot_removing_retry_state(&fixture, &removing)?,
        before,
        "retry mutated Git-admin replacement or cleanup-pending state"
    );
    Ok(())
}

#[test]
fn finalized_sidecar_replacement_preserves_tombstone_and_proof_ref() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let relocated_sidecar = fixture
        ._directory
        .path()
        .join("relocated-exact-sidecar.json");
    let sentinel = b"valuable replacement sidecar bytes\n".to_vec();
    let mut hook = ReplaceSidecarAtFinalized {
        store: &fixture.store,
        lookup: &fixture.lookup,
        relocated_sidecar: relocated_sidecar.clone(),
        sentinel: sentinel.clone(),
        tripped: false,
    };
    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut hook,
    )
    .expect_err("replacement sidecar must stop finalized cleanup");
    assert!(hook.tripped);
    assert!(
        error.contains("manifest sidecar"),
        "unexpected sidecar replacement rejection: {error}"
    );

    let removed = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "sidecar replacement lost the removal tombstone".to_string())?;
    if !matches!(removed, CodeWorktreeRemovalRecord::Removed(_)) {
        return Err("sidecar replacement did not preserve a Removed tombstone".to_string());
    }
    let authority = removed.authority();
    let sidecar = fixture
        .app_data
        .join("code")
        .join("removal-manifests-v1")
        .join(format!("{}.json", authority.physical_manifest_digest));
    assert_eq!(
        fs::read(&sidecar).map_err(|error| error.to_string())?,
        sentinel,
        "cleanup changed the replacement sidecar"
    );
    let exact_sidecar_before = fs::read(&relocated_sidecar)
        .map_err(|error| format!("relocated exact sidecar was not preserved: {error}"))?;
    let proof_ref = format!("refs/schoolx/removal-claims/{}", authority.removal_id);
    assert_eq!(
        git_line(
            &fixture.source_root,
            &["show-ref", "--verify", "--hash", &proof_ref],
        )?,
        authority.merge_proof.target_commit,
        "cleanup removed the proof ref despite a replacement sidecar"
    );

    let reopened = CodeThreadBindingStore::for_app_data(&fixture.app_data)?;
    let retry_error = unix::recover_all(&reopened, &fixture.nest_root, &mut unix::NoopFaultHook)
        .expect_err("replacement sidecar must keep cleanup fail closed");
    assert!(
        retry_error.contains("manifest sidecar"),
        "unexpected replacement-sidecar retry error: {retry_error}"
    );
    assert_eq!(
        fs::read(&sidecar).map_err(|error| error.to_string())?,
        sentinel
    );
    assert_eq!(
        fs::read(&relocated_sidecar).map_err(|error| error.to_string())?,
        exact_sidecar_before
    );
    assert_eq!(
        git_line(
            &fixture.source_root,
            &["show-ref", "--verify", "--hash", &proof_ref],
        )?,
        authority.merge_proof.target_commit
    );
    assert_eq!(
        reopened.lookup_worktree_removal(&fixture.lookup)?,
        Some(removed),
        "replacement-sidecar retry changed the tombstone"
    );
    Ok(())
}

#[test]
fn finalized_offline_common_dir_defers_then_converges_cleanup() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let mut fault = FailOnceAt {
        target: unix::FaultBoundary::Finalized,
        tripped: false,
    };
    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut fault,
    )
    .expect_err("finalized boundary fault should interrupt cleanup");
    assert!(error.contains("injected crash after Finalized"));
    assert!(fault.tripped);

    let removed = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "finalized fault lost the removal tombstone".to_string())?;
    if !matches!(removed, CodeWorktreeRemovalRecord::Removed(_)) {
        return Err("finalized fault did not preserve a Removed tombstone".to_string());
    }
    let authority = removed.authority();
    let common_dir = Path::new(&authority.physical.git_admin_parent)
        .parent()
        .ok_or_else(|| "Git-admin parent has no common directory".to_string())?
        .to_path_buf();
    let offline_common_dir = fixture._directory.path().join("offline-common-dir");
    let sidecar = fixture
        .app_data
        .join("code")
        .join("removal-manifests-v1")
        .join(format!("{}.json", authority.physical_manifest_digest));
    let sidecar_before = fs::read(&sidecar).map_err(|error| error.to_string())?;
    let proof_ref_relative = Path::new("refs")
        .join("schoolx")
        .join("removal-claims")
        .join(&authority.removal_id);
    fs::rename(&common_dir, &offline_common_dir)
        .map_err(|error| format!("failed to take common directory offline: {error}"))?;
    assert!(offline_common_dir.join(&proof_ref_relative).is_file());

    let reopened = CodeThreadBindingStore::for_app_data(&fixture.app_data)?;
    unix::recover_all(&reopened, &fixture.nest_root, &mut unix::NoopFaultHook)?;
    assert_eq!(
        fs::read(&sidecar).map_err(|error| error.to_string())?,
        sidecar_before,
        "offline cleanup changed its durable sidecar"
    );
    assert!(
        offline_common_dir.join(&proof_ref_relative).is_file(),
        "offline cleanup changed the inaccessible proof ref"
    );
    assert_eq!(
        reopened.lookup_worktree_removal(&fixture.lookup)?,
        Some(removed.clone())
    );

    fs::rename(&offline_common_dir, &common_dir)
        .map_err(|error| format!("failed to restore common directory: {error}"))?;
    unix::recover_all(&reopened, &fixture.nest_root, &mut unix::NoopFaultHook)?;
    assert_removed(&fixture, &removed)
}

#[test]
fn finalized_cleanup_preserves_replacement_proof_ref() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let mut fault = FailOnceAt {
        target: unix::FaultBoundary::Finalized,
        tripped: false,
    };
    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut fault,
    )
    .expect_err("finalized boundary fault should interrupt proof-ref cleanup");
    assert!(error.contains("injected crash after Finalized"));
    assert!(fault.tripped);

    let removed = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "finalized fault lost the removal tombstone".to_string())?;
    if !matches!(removed, CodeWorktreeRemovalRecord::Removed(_)) {
        return Err("finalized fault did not preserve a removed tombstone".to_string());
    }
    let authority = removed.authority();
    let proof_ref = format!("refs/schoolx/removal-claims/{}", authority.removal_id);
    let replacement = git_line(&fixture.source_root, &["rev-parse", "HEAD^{tree}"])?;
    if replacement == authority.merge_proof.target_commit {
        return Err("replacement proof-ref object unexpectedly equals target commit".to_string());
    }
    test_git(
        &fixture.source_root,
        &[
            "update-ref",
            &proof_ref,
            &replacement,
            &authority.merge_proof.target_commit,
        ],
    )?;
    let sidecar = fixture
        .app_data
        .join("code")
        .join("removal-manifests-v1")
        .join(format!("{}.json", authority.physical_manifest_digest));
    assert!(sidecar.is_file(), "finalized cleanup sidecar was missing");

    let reopened = CodeThreadBindingStore::for_app_data(&fixture.app_data)?;
    let recovery_error = unix::recover_all(&reopened, &fixture.nest_root, &mut unix::NoopFaultHook)
        .expect_err("cleanup must reject a different-OID proof-ref replacement");
    assert!(
        recovery_error.contains("proof ref replacement was preserved during cleanup"),
        "unexpected cleanup rejection: {recovery_error}"
    );
    assert_eq!(
        git_line(
            &fixture.source_root,
            &["show-ref", "--verify", "--hash", &proof_ref],
        )?,
        replacement,
        "cleanup changed a replacement proof ref"
    );
    assert!(sidecar.is_file(), "cleanup removed its retry sidecar");
    assert_eq!(
        reopened.lookup_worktree_removal(&fixture.lookup)?,
        Some(removed),
        "cleanup changed the finalized removal tombstone"
    );
    Ok(())
}

#[test]
fn finalized_cleanup_rejects_and_preserves_symbolic_proof_ref() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let mut fault = FailOnceAt {
        target: unix::FaultBoundary::Finalized,
        tripped: false,
    };
    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut fault,
    )
    .expect_err("finalized boundary fault should interrupt proof-ref cleanup");
    assert!(error.contains("injected crash after Finalized"));
    assert!(fault.tripped);

    let removed = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "finalized fault lost the removal tombstone".to_string())?;
    if !matches!(removed, CodeWorktreeRemovalRecord::Removed(_)) {
        return Err("finalized fault did not preserve a Removed tombstone".to_string());
    }
    let authority = removed.authority();
    let proof_ref = format!("refs/schoolx/removal-claims/{}", authority.removal_id);
    let target_ref = authority.merge_proof.target_ref.clone();
    let sidecar = fixture
        .app_data
        .join("code")
        .join("removal-manifests-v1")
        .join(format!("{}.json", authority.physical_manifest_digest));
    let sidecar_before = fs::read(&sidecar).map_err(|error| error.to_string())?;
    test_git(
        &fixture.source_root,
        &["symbolic-ref", &proof_ref, &target_ref],
    )?;
    assert_eq!(
        git_line(&fixture.source_root, &["symbolic-ref", &proof_ref])?,
        target_ref
    );
    assert_eq!(
        git_line(
            &fixture.source_root,
            &["show-ref", "--verify", "--hash", &proof_ref],
        )?,
        authority.merge_proof.target_commit,
        "symbolic replacement did not resolve to the authorized target OID"
    );

    let reopened = CodeThreadBindingStore::for_app_data(&fixture.app_data)?;
    let recovery_error = unix::recover_all(&reopened, &fixture.nest_root, &mut unix::NoopFaultHook)
        .expect_err("cleanup must reject symbolic proof-ref authority");
    assert!(
        recovery_error.contains("proof ref is symbolic or has ambiguous raw authority"),
        "unexpected symbolic-ref cleanup rejection: {recovery_error}"
    );
    assert_eq!(
        git_line(&fixture.source_root, &["symbolic-ref", &proof_ref])?,
        target_ref,
        "cleanup changed the symbolic proof ref"
    );
    assert_eq!(
        fs::read(&sidecar).map_err(|error| error.to_string())?,
        sidecar_before,
        "cleanup changed the sidecar after symbolic-ref rejection"
    );
    assert_eq!(
        reopened.lookup_worktree_removal(&fixture.lookup)?,
        Some(removed),
        "symbolic-ref cleanup changed the removal tombstone"
    );
    Ok(())
}

#[test]
fn finalized_cleanup_preserves_loose_proof_ref_symlink_replacement() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    let sibling_before = snapshot_tree(&fixture.sibling_root)?;
    let transcript_before = fs::read(&fixture.transcript).map_err(|error| error.to_string())?;
    let mut fault = FailOnceAt {
        target: unix::FaultBoundary::Finalized,
        tripped: false,
    };
    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut fault,
    )
    .expect_err("finalized boundary fault should interrupt proof-ref cleanup");
    assert!(error.contains("injected crash after Finalized"));
    assert!(fault.tripped);

    let removed = fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .ok_or_else(|| "finalized fault lost the removal tombstone".to_string())?;
    if !matches!(removed, CodeWorktreeRemovalRecord::Removed(_)) {
        return Err("finalized fault did not preserve a removed tombstone".to_string());
    }
    let authority = removed.authority();
    let loose_ref = fixture
        .source_root
        .join(".git")
        .join("refs")
        .join("schoolx")
        .join("removal-claims")
        .join(&authority.removal_id);
    let loose_before = fs::read(&loose_ref)
        .map_err(|error| format!("finalized proof ref was not a loose file: {error}"))?;
    let expected_bytes = format!("{}\n", authority.merge_proof.target_commit).into_bytes();
    assert_eq!(loose_before, expected_bytes);
    let replacement_target = fixture._directory.path().join("replacement-proof-ref.txt");
    fs::write(&replacement_target, &expected_bytes).map_err(|error| error.to_string())?;
    fs::remove_file(&loose_ref).map_err(|error| error.to_string())?;
    std::os::unix::fs::symlink(&replacement_target, &loose_ref)
        .map_err(|error| format!("failed to install loose proof-ref symlink: {error}"))?;
    let sidecar = fixture
        .app_data
        .join("code")
        .join("removal-manifests-v1")
        .join(format!("{}.json", authority.physical_manifest_digest));
    assert!(sidecar.is_file(), "finalized cleanup sidecar was missing");

    let reopened = CodeThreadBindingStore::for_app_data(&fixture.app_data)?;
    let recovery_error = unix::recover_all(&reopened, &fixture.nest_root, &mut unix::NoopFaultHook)
        .expect_err("cleanup must reject a filesystem-symlink proof-ref replacement");
    assert!(
        recovery_error.contains("failed to pin manifest file"),
        "unexpected cleanup rejection: {recovery_error}"
    );
    assert!(
        fs::symlink_metadata(&loose_ref)
            .map_err(|error| error.to_string())?
            .file_type()
            .is_symlink(),
        "cleanup replaced or removed the loose proof-ref symlink"
    );
    assert_eq!(
        fs::read_link(&loose_ref).map_err(|error| error.to_string())?,
        replacement_target
    );
    assert_eq!(
        fs::read(&replacement_target).map_err(|error| error.to_string())?,
        expected_bytes,
        "cleanup changed the symlink target bytes"
    );
    assert!(sidecar.is_file(), "cleanup removed its retry sidecar");
    assert_eq!(
        reopened.lookup_worktree_removal(&fixture.lookup)?,
        Some(removed),
        "cleanup changed the finalized removal tombstone"
    );
    assert_eq!(snapshot_tree(&fixture.sibling_root)?, sibling_before);
    assert_eq!(
        fs::read(&fixture.transcript).map_err(|error| error.to_string())?,
        transcript_before
    );
    Ok(())
}

#[test]
fn assume_unchanged_content_drift_is_not_deletion_authority() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    test_git(
        &fixture.managed_root,
        &["update-index", "--assume-unchanged", "--", "README.md"],
    )?;
    fs::write(
        fixture.managed_root.join("README.md"),
        b"locally valuable bytes hidden from status\n",
    )
    .map_err(|error| error.to_string())?;
    assert!(test_git(&fixture.managed_root, &["status", "--porcelain=v1", "-z"])?.is_empty());
    let root_before = snapshot_tree(&fixture.managed_root)?;
    let admin_before = snapshot_tree(
        linked_admin_entry(&fixture.managed_root)?
            .parent()
            .ok_or_else(|| "Git-admin entry has no parent".to_string())?,
    )?;

    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut unix::NoopFaultHook,
    )
    .expect_err("assume-unchanged content drift must reject removal");

    assert!(
        error.contains("does not match the exact Git object"),
        "unexpected rejection: {error}"
    );
    assert!(fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .is_none());
    assert_eq!(snapshot_tree(&fixture.managed_root)?, root_before);
    assert_eq!(
        snapshot_tree(
            linked_admin_entry(&fixture.managed_root)?
                .parent()
                .ok_or_else(|| "Git-admin entry has no parent".to_string())?,
        )?,
        admin_before
    );
    assert!(!fixture
        .app_data
        .join("code")
        .join("removal-manifests-v1")
        .exists());
    Ok(())
}

#[test]
fn ignored_entry_rejects_claim_with_zero_git_store_or_filesystem_mutation() -> Result<(), String> {
    let fixture = prepare_fixture()?;
    fs::write(
        fixture.managed_root.join("ignored-output"),
        b"must never be inferred as disposable\n",
    )
    .map_err(|error| error.to_string())?;
    assert!(
        test_git(&fixture.managed_root, &["status", "--porcelain=v1", "-z"],)?.is_empty(),
        "fixture ignored entry unexpectedly appeared in ordinary status"
    );

    let root_parent = fixture
        .managed_root
        .parent()
        .ok_or_else(|| "managed root has no parent".to_string())?;
    let admin_entry = linked_admin_entry(&fixture.managed_root)?;
    let admin_parent = admin_entry
        .parent()
        .ok_or_else(|| "Git-admin entry has no parent".to_string())?;
    let store_before = fs::read(fixture.store.store_path()).map_err(|error| error.to_string())?;
    let store_modified_before = fs::metadata(fixture.store.store_path())
        .and_then(|metadata| metadata.modified())
        .map_err(|error| error.to_string())?;
    let root_parent_before = snapshot_tree(root_parent)?;
    let admin_parent_before = snapshot_tree(admin_parent)?;
    let sibling_before = snapshot_tree(&fixture.sibling_root)?;
    let transcript_before = fs::read(&fixture.transcript).map_err(|error| error.to_string())?;
    let refs_before = test_git(
        &fixture.source_root,
        &["for-each-ref", "--format=%(refname) %(objectname)"],
    )?;

    let error = unix::claim_or_resume(
        &fixture.store,
        &fixture.lookup,
        &fixture.nest_root,
        Instant::now() + Duration::from_secs(120),
        &mut unix::NoopFaultHook,
    )
    .expect_err("ignored entry must reject physical-removal claim");
    assert!(
        error.contains("rejects unexpected worktree entry"),
        "unexpected rejection: {error}"
    );

    assert!(fixture
        .store
        .lookup_worktree_removal(&fixture.lookup)?
        .is_none());
    assert_eq!(
        fs::read(fixture.store.store_path()).map_err(|error| error.to_string())?,
        store_before,
        "binding store bytes changed on rejected claim"
    );
    assert_eq!(
        fs::metadata(fixture.store.store_path())
            .and_then(|metadata| metadata.modified())
            .map_err(|error| error.to_string())?,
        store_modified_before,
        "binding store mtime changed on rejected claim"
    );
    assert_eq!(snapshot_tree(root_parent)?, root_parent_before);
    assert_eq!(snapshot_tree(admin_parent)?, admin_parent_before);
    assert_eq!(snapshot_tree(&fixture.sibling_root)?, sibling_before);
    assert_eq!(
        fs::read(&fixture.transcript).map_err(|error| error.to_string())?,
        transcript_before
    );
    assert_eq!(
        test_git(
            &fixture.source_root,
            &["for-each-ref", "--format=%(refname) %(objectname)"],
        )?,
        refs_before,
        "Git refs changed on rejected claim"
    );
    assert!(
        !fixture
            .app_data
            .join("code")
            .join("removal-manifests-v1")
            .exists(),
        "rejected claim persisted a manifest sidecar"
    );
    Ok(())
}

/// Child-process entry that terminates at one durable physical-removal boundary.
#[test]
#[ignore]
fn physical_removal_crash_subprocess_entry() {
    if let Err(error) = run_crash_child_from_env() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

/// Child-process entry used by the typed removal Git helper during unit tests.
#[test]
#[ignore]
fn removal_git_helper_subprocess_entry() {
    if let Err(error) = unix::execute_helper() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
