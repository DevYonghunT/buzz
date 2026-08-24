use super::{file_identity::*, git_helper::*, manifest_store::*, process::*, *};

pub(super) fn path_string(path: &Path, label: &str) -> Result<String, String> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("{label} is not valid UTF-8"))
}

pub(super) fn zero_oid_for(commit: &str) -> Result<&'static str, String> {
    match commit.len() {
        40 => Ok(ZERO_SHA1),
        64 => Ok(ZERO_SHA256),
        _ => Err("SchoolX Code removal proof has an unsupported object-id length".to_string()),
    }
}

pub(super) fn read_proof_ref(
    launch: &RemovalGitLaunchAuthority,
    common_dir: &fs::File,
    common_dir_path: &Path,
    authority: &super::super::super::CodeWorktreeRemovalAuthority,
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

pub(super) fn ensure_proof_ref(
    launch: &RemovalGitLaunchAuthority,
    common_dir: &fs::File,
    common_dir_path: &Path,
    authority: &super::super::super::CodeWorktreeRemovalAuthority,
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

pub(super) fn require_exact_proof_ref(
    launch: &RemovalGitLaunchAuthority,
    common_dir: &fs::File,
    common_dir_path: &Path,
    authority: &super::super::super::CodeWorktreeRemovalAuthority,
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

pub(super) fn delete_proof_ref_if_matches(
    launch: &RemovalGitLaunchAuthority,
    common_dir: &fs::File,
    common_dir_path: &Path,
    authority: &super::super::super::CodeWorktreeRemovalAuthority,
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

pub(super) fn sync_exact_proof_ref(
    common_dir: &fs::File,
    authority: &super::super::super::CodeWorktreeRemovalAuthority,
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

pub(super) fn sync_deleted_proof_ref(
    common_dir: &fs::File,
    authority: &super::super::super::CodeWorktreeRemovalAuthority,
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
