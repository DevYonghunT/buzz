use super::{delete::*, manifest_capture::*, manifest_store::*, *};

pub(super) fn read_regular_identity_at(
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

pub(super) fn read_symlink_identity_at(
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

pub(super) fn git_blob_oid_regular_at(
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

pub(super) fn hash_regular_git_blob<D>(
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

pub(super) fn git_blob_oid_symlink_at(
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

pub(super) fn hash_git_blob_bytes<D>(bytes: &[u8]) -> String
where
    D: Digest + Default,
{
    let mut hasher = D::default();
    hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

pub(super) fn read_small_regular_at(
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

pub(super) fn open_directory_absolute(path: &Path, label: &str) -> Result<fs::File, String> {
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

pub(super) fn open_directory_at(
    parent: &fs::File,
    name: &OsStr,
    label: &str,
) -> Result<fs::File, String> {
    let component =
        CString::new(name.as_bytes()).map_err(|_| format!("{label} contains an interior NUL"))?;
    open_directory_at_cstr(parent, component.as_c_str(), label)
}

pub(super) fn open_optional_directory_at(
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

pub(super) fn open_directory_at_cstr(
    parent: &fs::File,
    name: &CStr,
    label: &str,
) -> Result<fs::File, String> {
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

pub(super) fn open_expected_directory_at(
    parent: &fs::File,
    name: &OsStr,
    expected: &NodeIdentity,
    label: &str,
) -> Result<fs::File, String> {
    let component =
        CString::new(name.as_bytes()).map_err(|_| format!("{label} contains an interior NUL"))?;
    open_expected_directory_at_cstr(parent, component.as_c_str(), expected, label)
}

pub(super) fn open_expected_directory_at_cstr(
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

pub(super) fn named_directory_state(
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

pub(super) fn verify_named_directory(
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
