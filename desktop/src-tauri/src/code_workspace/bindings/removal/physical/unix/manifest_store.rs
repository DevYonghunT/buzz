use super::{delete::*, file_identity::*, scan::*, *};

pub(super) fn persist_manifest_sidecar(
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

pub(super) fn unlink_manifest_temp(directory: &fs::File, name: &str) -> Result<(), String> {
    let component = CString::new(name.as_bytes())
        .map_err(|_| "removal manifest temp name contains NUL".to_string())?;
    match rustix::fs::unlinkat(directory, component.as_c_str(), AtFlags::empty()) {
        Ok(()) | Err(rustix::io::Errno::NOENT) => Ok(()),
        Err(error) => Err(format!("failed to remove removal manifest temp: {error}")),
    }
}

pub(super) fn remove_manifest_sidecar(
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

pub(super) fn load_manifest_sidecar(
    store: &CodeThreadBindingStore,
    authority: &super::super::super::CodeWorktreeRemovalAuthority,
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

pub(super) fn open_manifest_directory(
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

pub(super) fn verify_manifest_directory_named(
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

pub(super) fn harden_manifest_absence(
    store: &CodeThreadBindingStore,
    digest: &str,
) -> Result<(), String> {
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

pub(super) fn ensure_manifest_directory(
    store: &CodeThreadBindingStore,
) -> Result<ManifestDirectory, String> {
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

pub(super) fn read_manifest_file_at(
    directory: &fs::File,
    name: &str,
) -> Result<ManifestFileRead, String> {
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

pub(super) fn verify_manifest_file_named(
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

pub(super) fn snapshot_named_siblings(
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

pub(super) fn verify_sibling_snapshot(
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

pub(super) fn node_identity_from_parts(
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

pub(super) fn node_identity_from_fd(
    file: &fs::File,
    stat: &rustix::fs::Stat,
    digest: Option<String>,
) -> Result<NodeIdentity, String> {
    node_identity_from_parts(stat, incarnation_from_fd(file, stat)?, digest)
}

pub(super) fn node_identity_from_at(
    parent: &fs::File,
    name: &CStr,
    stat: &rustix::fs::Stat,
    digest: Option<String>,
) -> Result<NodeIdentity, String> {
    node_identity_from_parts(stat, incarnation_from_at(parent, name, stat)?, digest)
}

#[cfg(target_os = "linux")]
pub(super) fn incarnation_from_fd(
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
pub(super) fn incarnation_from_at(
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
pub(super) fn validate_linux_incarnation(
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
pub(super) fn incarnation_from_fd(
    _file: &fs::File,
    stat: &rustix::fs::Stat,
) -> Result<(i64, u32, u64), String> {
    macos_incarnation(stat)
}

#[cfg(target_os = "macos")]
pub(super) fn incarnation_from_at(
    _parent: &fs::File,
    _name: &CStr,
    stat: &rustix::fs::Stat,
) -> Result<(i64, u32, u64), String> {
    macos_incarnation(stat)
}

#[cfg(target_os = "macos")]
pub(super) fn macos_incarnation(stat: &rustix::fs::Stat) -> Result<(i64, u32, u64), String> {
    let nanoseconds = u32::try_from(stat.st_birthtime_nsec)
        .map_err(|_| "SchoolX Code removal birth-time nanoseconds are invalid".to_string())?;
    Ok((stat.st_birthtime, nanoseconds, stat.st_gen as u64))
}

pub(super) fn directory_identity(directory: &fs::File) -> Result<NodeIdentity, String> {
    let stat = rustix::fs::fstat(directory)
        .map_err(|error| format!("failed to inspect pinned removal directory: {error}"))?;
    if !FileType::from_raw_mode(stat.st_mode).is_dir() {
        return Err("pinned SchoolX Code removal handle is not a directory".to_string());
    }
    node_identity_from_fd(directory, &stat, None)
}

#[cfg(target_os = "linux")]
pub(super) fn mount_id(file: &fs::File) -> Result<u64, String> {
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
pub(super) fn mount_id(file: &fs::File) -> Result<u64, String> {
    let stat = rustix::fs::fstatvfs(file)
        .map_err(|error| format!("failed to inspect removal mount identity: {error}"))?;
    if stat.f_fsid == 0 {
        return Err("SchoolX Code removal requires macOS mount identity".to_string());
    }
    Ok(stat.f_fsid)
}

pub(super) fn same_directory_identity(left: &NodeIdentity, right: &NodeIdentity) -> bool {
    left.device == right.device
        && left.inode == right.inode
        && left.mode == right.mode
        && left.birth_time_seconds == right.birth_time_seconds
        && left.birth_time_nanoseconds == right.birth_time_nanoseconds
        && left.generation == right.generation
        && FileType::from_raw_mode(left.mode as _).is_dir()
        && FileType::from_raw_mode(right.mode as _).is_dir()
}

pub(super) fn same_named_identity(left: &NodeIdentity, right: &NodeIdentity) -> bool {
    left.device == right.device
        && left.inode == right.inode
        && left.mode == right.mode
        && left.size == right.size
        && left.birth_time_seconds == right.birth_time_seconds
        && left.birth_time_nanoseconds == right.birth_time_nanoseconds
        && left.generation == right.generation
        && left.content_sha256 == right.content_sha256
}

pub(super) fn require_same_mount(
    parent: &fs::File,
    child: &fs::File,
    label: &str,
) -> Result<(), String> {
    let parent_identity = directory_identity(parent)?;
    let child_identity = directory_identity(child)?;
    if parent_identity.device != child_identity.device || mount_id(parent)? != mount_id(child)? {
        return Err(format!("{label} crosses a nested mount boundary"));
    }
    Ok(())
}
