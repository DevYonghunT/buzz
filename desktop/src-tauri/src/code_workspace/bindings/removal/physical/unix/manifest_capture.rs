use super::{
    delete::*, file_identity::*, manifest_store::*, process::*, proof_refs::*, scan::*, *,
};

pub(super) fn capture_manifest(
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

pub(super) fn capture_manifest_for_binding(
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

pub(super) fn pin_layout(
    binding: &CodeThreadBinding,
    nest_root: &Path,
) -> Result<PinnedLayout, String> {
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

pub(super) fn verify_pinned_layout(
    layout: &PinnedLayout,
    binding: &CodeThreadBinding,
) -> Result<(), String> {
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

pub(super) struct ReadFile {
    pub(super) bytes: Vec<u8>,
}

pub(super) fn verify_admin_reciprocal(
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

pub(super) fn verify_admin_head(
    layout: &PinnedLayout,
    proof: &CodeMergeProofReceipt,
) -> Result<(), String> {
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

pub(super) fn verify_repository_storage(
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

pub(super) fn verify_owned_object_storage_tree(
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

pub(super) fn verify_owned_regular_file_at(
    parent: &fs::File,
    name: &CStr,
    label: &str,
) -> Result<(), String> {
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

pub(super) fn read_tracked_index(
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

pub(super) fn verify_local_blob_objects(
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

pub(super) fn parse_index_entries(
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

pub(super) fn parse_head_entries(
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

pub(super) fn parse_tracked_kind(mode: &str) -> Result<TrackedKind, String> {
    match mode {
        "100644" => Ok(TrackedKind::Regular { executable: false }),
        "100755" => Ok(TrackedKind::Regular { executable: true }),
        "120000" => Ok(TrackedKind::Symlink),
        "160000" => Err("SchoolX Code removal refuses submodule/gitlink entries".to_string()),
        _ => Err(format!("SchoolX Code removal rejects tracked mode {mode}")),
    }
}
