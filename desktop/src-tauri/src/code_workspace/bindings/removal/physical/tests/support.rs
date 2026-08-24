use super::*;

pub(super) struct PhysicalFixture {
    pub(super) _directory: tempfile::TempDir,
    pub(super) app_data: PathBuf,
    pub(super) nest_root: PathBuf,
    pub(super) source_root: PathBuf,
    pub(super) managed_root: PathBuf,
    pub(super) sibling_root: PathBuf,
    pub(super) transcript: PathBuf,
    pub(super) external_symlink_target: PathBuf,
    pub(super) store: CodeThreadBindingStore,
    pub(super) lookup: CodeThreadBindingLookupInput,
}

#[cfg(target_os = "linux")]
const LINUX_MOUNT_MODE_ENV: &str = "SCHOOLX_CODE_REMOVAL_TEST_MOUNT_MODE";

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
pub(super) enum LinuxMountMode {
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
pub(super) fn run_linux_mount_utility(
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
pub(super) struct LinuxNodeProbe {
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
pub(super) struct LinuxSelfBindMount {
    mode: LinuxMountMode,
    target: PathBuf,
    original: LinuxNodeProbe,
    mounted: bool,
}

#[cfg(target_os = "linux")]
impl LinuxSelfBindMount {
    pub(super) fn install(target: &Path) -> Result<Self, String> {
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
pub(super) fn finish_linux_mounted_test(
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
pub(super) enum TestRepositoryStorage {
    SelfContained,
    SharedClone,
}

pub(super) fn test_git(cwd: &Path, arguments: &[&str]) -> Result<Vec<u8>, String> {
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

pub(super) fn git_line(cwd: &Path, arguments: &[&str]) -> Result<String, String> {
    let stdout = test_git(cwd, arguments)?;
    let value = std::str::from_utf8(&stdout)
        .map_err(|error| format!("test Git output was not UTF-8: {error}"))?
        .trim_end_matches(['\r', '\n']);
    if value.is_empty() || value.contains(['\r', '\n']) {
        return Err("test Git output was not exactly one line".to_string());
    }
    Ok(value.to_string())
}

pub(super) fn path_string(path: &Path, label: &str) -> Result<String, String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{label} was not UTF-8"))
}

pub(super) fn archive(
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

pub(super) fn prepare_linked_worktree(
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

pub(super) fn initialize_test_repository(
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

pub(super) fn prepare_fixture_with_storage(
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

pub(super) fn prepare_fixture() -> Result<PhysicalFixture, String> {
    prepare_fixture_with_storage(TestRepositoryStorage::SelfContained)
}

pub(super) fn snapshot_tree(root: &Path) -> Result<BTreeMap<String, Vec<u8>>, String> {
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

pub(super) fn linked_admin_entry(managed_root: &Path) -> Result<PathBuf, String> {
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

pub(super) fn assert_claim_rejected_without_mutation(
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

pub(super) fn assert_removed(
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
