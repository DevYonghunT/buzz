use super::{delete::*, file_identity::*, manifest_store::*, process::*, proof_refs::*, *};

pub(super) fn require_clean_worktree(
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

pub(super) fn scan_managed_root(
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
pub(super) fn scan_root_directory(
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

pub(super) fn scan_admin_tree(root: &fs::File) -> Result<Vec<ManifestEntry>, String> {
    let root_identity = directory_identity(root)?;
    let root_device = root_identity.device;
    let root_mount_id = mount_id(root)?;
    let mut entries = Vec::new();
    scan_admin_directory(root, Vec::new(), root_device, root_mount_id, &mut entries)?;
    sort_manifest_entries(&mut entries);
    Ok(entries)
}

pub(super) fn require_admin_authority_entries(entries: &[ManifestEntry]) -> Result<(), String> {
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

pub(super) fn scan_admin_directory(
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

pub(super) fn validate_manifest(manifest: &PhysicalRemovalManifest) -> Result<(), String> {
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

pub(super) fn validate_node_identity(
    identity: &NodeIdentity,
    directory: bool,
) -> Result<(), String> {
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

pub(super) fn validate_named_identities(values: &[NamedIdentity]) -> Result<(), String> {
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

pub(super) fn validate_manifest_entries(
    entries: &[ManifestEntry],
    root_tree: bool,
) -> Result<(), String> {
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

pub(super) fn canonical_manifest_bytes(
    manifest: &PhysicalRemovalManifest,
) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(manifest)
        .map_err(|error| format!("failed to encode SchoolX Code removal manifest: {error}"))?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(format!(
            "SchoolX Code removal manifest exceeds {MAX_MANIFEST_BYTES} bytes"
        ));
    }
    Ok(bytes)
}
