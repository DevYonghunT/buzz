use super::*;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum CrashBoundary {
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
    pub(super) const ALL: [Self; 11] = [
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
pub(super) struct CrashRequest {
    version: u32,
    app_data: String,
    nest_root: String,
    lookup: CodeThreadBindingLookupInput,
    boundary: CrashBoundary,
}

pub(super) struct ExitProcessAt {
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

pub(super) fn crash_request_for(
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

pub(super) fn spawn_crash_child(request: &CrashRequest) -> Result<(), String> {
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

pub(super) fn run_crash_child_from_env() -> Result<(), String> {
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

pub(super) struct FailOnceAt {
    pub(super) target: unix::FaultBoundary,
    pub(super) tripped: bool,
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
pub(super) fn prepare_durable_removing(
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
pub(super) fn assert_positive_birth_time_identities(
    value: &serde_json::Value,
) -> Result<usize, String> {
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
pub(super) fn linux_removing_crash_reopen_recovers_positive_birth_time_identities(
) -> Result<(), String> {
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

pub(super) struct DeleteFutureRootEntryAtQuarantine<'a> {
    pub(super) store: &'a CodeThreadBindingStore,
    pub(super) lookup: &'a CodeThreadBindingLookupInput,
    pub(super) tripped: bool,
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

pub(super) struct InstallOriginalReplacementAtQuarantine {
    pub(super) original: PathBuf,
    pub(super) sentinel: Vec<u8>,
    pub(super) tripped: bool,
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

pub(super) struct ReplaceGitAdminAtRootDeleted<'a> {
    pub(super) store: &'a CodeThreadBindingStore,
    pub(super) lookup: &'a CodeThreadBindingLookupInput,
    pub(super) relocated_admin: PathBuf,
    pub(super) sentinel: Vec<u8>,
    pub(super) tripped: bool,
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

pub(super) struct ReplaceQuarantineWithSiblingAtQuarantined<'a> {
    pub(super) store: &'a CodeThreadBindingStore,
    pub(super) lookup: &'a CodeThreadBindingLookupInput,
    pub(super) relocated_name: String,
    pub(super) sentinel: Vec<u8>,
    pub(super) tripped: bool,
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

pub(super) struct ReplaceSidecarAtFinalized<'a> {
    pub(super) store: &'a CodeThreadBindingStore,
    pub(super) lookup: &'a CodeThreadBindingLookupInput,
    pub(super) relocated_sidecar: PathBuf,
    pub(super) sentinel: Vec<u8>,
    pub(super) tripped: bool,
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
pub(super) struct RemovingRetrySnapshot {
    store_bytes: Vec<u8>,
    store_modified: std::time::SystemTime,
    root_parent: BTreeMap<String, Vec<u8>>,
    admin_parent: BTreeMap<String, Vec<u8>>,
    manifest_sidecars: BTreeMap<String, Vec<u8>>,
    sibling: BTreeMap<String, Vec<u8>>,
    transcript: Vec<u8>,
    refs: Vec<u8>,
}

pub(super) fn snapshot_removing_retry_state(
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
pub(super) fn assert_linux_claim_self_bind_rejected(
    fixture: &PhysicalFixture,
    target: &Path,
    expected_error: &str,
) -> Result<(), String> {
    let mut mount = LinuxSelfBindMount::install(target)?;
    let result = assert_claim_rejected_without_mutation(fixture, expected_error);
    finish_linux_mounted_test(result, &mut mount)
}

#[cfg(target_os = "linux")]
pub(super) fn assert_linux_removing_self_bind_rejected(
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
