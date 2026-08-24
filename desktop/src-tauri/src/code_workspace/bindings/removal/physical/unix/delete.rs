use super::{file_identity::*, manifest_capture::*, manifest_store::*, proof_refs::*, *};

pub(super) fn pin_recovery_boundary(
    authority: &super::super::super::CodeWorktreeRemovalAuthority,
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

pub(super) fn verify_recovery_boundary_paths(
    boundary: &RecoveryBoundary,
    authority: &super::super::super::CodeWorktreeRemovalAuthority,
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

pub(super) fn verify_layout_against_authority(
    layout: &PinnedLayout,
    authority: &super::super::super::CodeWorktreeRemovalAuthority,
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

pub(super) fn observe_coordinates(
    boundary: &RecoveryBoundary,
    authority: &super::super::super::CodeWorktreeRemovalAuthority,
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

pub(super) fn quarantine_root(
    root_parent: &fs::File,
    authority: &super::super::super::CodeWorktreeRemovalAuthority,
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

pub(super) fn delete_manifest_tree(
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

pub(super) fn verify_known_deletion_prefix(
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

pub(super) fn manifest_entry_is_present(
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

pub(super) fn verify_manifest_tree_state(
    root: &fs::File,
    entries: &[ManifestEntry],
) -> Result<(), String> {
    let expected = entries
        .iter()
        .map(|entry| Ok((decode_hex_path(&entry.path_hex)?, entry)))
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    verify_directory_entries_recursive(root, Vec::new(), &expected)
}

pub(super) fn verify_directory_entries_recursive(
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

pub(super) fn delete_manifest_entry(
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

pub(super) fn verify_entry_identity(
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

pub(super) fn verify_directory_empty(directory: &fs::File) -> Result<(), String> {
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

pub(super) fn remove_named_root(
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

pub(super) fn verify_final_absence_and_siblings(
    boundary: &RecoveryBoundary,
    authority: &super::super::super::CodeWorktreeRemovalAuthority,
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

pub(super) fn parse_gitdir_file(bytes: &[u8], root: &Path) -> Result<PathBuf, String> {
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

pub(super) fn parse_plain_path_file(bytes: &[u8], parent: &Path) -> Result<PathBuf, String> {
    let value = trim_one_line(bytes, "Git-admin path file")?;
    let path = PathBuf::from(OsString::from_vec(value.to_vec()));
    Ok(if path.is_absolute() {
        path
    } else {
        parent.join(path)
    })
}

pub(super) fn trim_one_line<'a>(bytes: &'a [u8], label: &str) -> Result<&'a [u8], String> {
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

pub(super) fn worktree_name(
    authority: &super::super::super::CodeWorktreeRemovalAuthority,
) -> Result<&OsStr, String> {
    let worktree_id = authority
        .binding
        .worktree_id
        .as_deref()
        .ok_or_else(|| "SchoolX Code removal authority lost its worktree id".to_string())?;
    Ok(OsStr::new(worktree_id))
}

pub(super) fn validate_safe_component(value: &OsStr, label: &str) -> Result<(), String> {
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

pub(super) fn validate_relative_bytes(path: &[u8]) -> Result<(), String> {
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

pub(super) fn split_relative_bytes(path: &[u8]) -> Result<Vec<&[u8]>, String> {
    validate_relative_bytes(path)?;
    Ok(path.split(|byte| *byte == b'/').collect())
}

pub(super) fn join_relative(prefix: &[u8], name: &[u8]) -> Vec<u8> {
    if prefix.is_empty() {
        return name.to_vec();
    }
    let mut result = Vec::with_capacity(prefix.len() + 1 + name.len());
    result.extend_from_slice(prefix);
    result.push(b'/');
    result.extend_from_slice(name);
    result
}

pub(super) fn join_components(components: &[&[u8]]) -> Vec<u8> {
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

pub(super) fn decode_hex_path(value: &str) -> Result<Vec<u8>, String> {
    if value.is_empty() || !value.len().is_multiple_of(2) {
        return Err("SchoolX Code removal manifest path encoding is invalid".to_string());
    }
    hex::decode(value).map_err(|error| format!("invalid removal manifest path encoding: {error}"))
}

pub(super) fn sort_manifest_entries(entries: &mut [ManifestEntry]) {
    entries.sort_by(|left, right| left.path_hex.cmp(&right.path_hex));
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}
