use std::fs;
use std::io::{Read, Seek};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in super::super) struct FileIdentity {
    pub(in super::super) path: String,
    pub(in super::super) device: u64,
    pub(in super::super) inode: u64,
    pub(in super::super) owner: u32,
    pub(in super::super) mode: u32,
    pub(in super::super) link_count: u64,
    pub(in super::super) size: u64,
    pub(in super::super) digest: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in super::super) struct DirectoryIdentity {
    pub(in super::super) path: String,
    pub(in super::super) device: u64,
    pub(in super::super) inode: u64,
    pub(in super::super) owner: u32,
    pub(in super::super) mode: u32,
    pub(in super::super) link_count: u64,
}

pub(in super::super) fn pin_input_file(
    path: &Path,
    max_bytes: usize,
) -> Result<FileIdentity, String> {
    pin_regular_file(path, max_bytes)
}

#[cfg(unix)]
pub(in super::super) fn pin_directory(path: &Path) -> Result<DirectoryIdentity, String> {
    use std::os::unix::fs::OpenOptionsExt as _;

    if !path.is_absolute() {
        return Err("Git directory evidence must be absolute".to_string());
    }
    let handle = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| format!("failed to pin Git directory evidence: {error}"))?;
    let identity = directory_identity(path, &handle)?;
    verify_directory_identity(&identity, &handle)?;
    Ok(identity)
}

#[cfg(not(unix))]
pub(in super::super) fn pin_directory(_path: &Path) -> Result<DirectoryIdentity, String> {
    Err("secure Git directory pinning is unavailable on this platform".to_string())
}

#[cfg(unix)]
pub(in super::super) fn verify_named_directory_identity(
    identity: &DirectoryIdentity,
) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt as _;

    let handle = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(&identity.path)
        .map_err(|error| format!("failed to reopen Git directory evidence: {error}"))?;
    verify_directory_identity(identity, &handle)
}

#[cfg(not(unix))]
pub(in super::super) fn verify_named_directory_identity(
    _identity: &DirectoryIdentity,
) -> Result<(), String> {
    Err("secure Git directory pinning is unavailable on this platform".to_string())
}

#[cfg(unix)]
pub(super) fn directory_identity(
    path: &Path,
    handle: &fs::File,
) -> Result<DirectoryIdentity, String> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = handle
        .metadata()
        .map_err(|error| format!("failed to inspect pinned directory: {error}"))?;
    if !metadata.is_dir() {
        return Err("pinned Git path is not a directory".to_string());
    }
    Ok(DirectoryIdentity {
        path: path
            .to_str()
            .ok_or_else(|| "pinned Git directory is not UTF-8".to_string())?
            .to_string(),
        device: metadata.dev(),
        inode: metadata.ino(),
        owner: metadata.uid(),
        mode: metadata.mode(),
        link_count: metadata.nlink(),
    })
}

#[cfg(unix)]
pub(super) fn verify_directory_identity(
    identity: &DirectoryIdentity,
    handle: &fs::File,
) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt as _;

    let named = fs::symlink_metadata(&identity.path)
        .map_err(|error| format!("failed to verify named Git directory: {error}"))?;
    let opened = handle
        .metadata()
        .map_err(|error| format!("failed to verify pinned Git directory: {error}"))?;
    if named.file_type().is_symlink()
        || !named.is_dir()
        || !opened.is_dir()
        || named.dev() != identity.device
        || named.ino() != identity.inode
        || opened.dev() != identity.device
        || opened.ino() != identity.inode
        || named.uid() != identity.owner
        || named.mode() != identity.mode
    {
        return Err("pinned Git directory moved or was replaced".to_string());
    }
    Ok(())
}

#[cfg(all(unix, test))]
pub(super) fn verify_named_directory(identity: &DirectoryIdentity) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = fs::symlink_metadata(&identity.path)
        .map_err(|error| format!("failed to verify named Git root: {error}"))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.dev() != identity.device
        || metadata.ino() != identity.inode
        || metadata.uid() != identity.owner
        || metadata.mode() != identity.mode
    {
        return Err("named Git root identity changed".to_string());
    }
    Ok(())
}

#[cfg(unix)]
pub(super) fn pin_regular_file(path: &Path, max_bytes: usize) -> Result<FileIdentity, String> {
    let mut file = open_no_follow(path)?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("failed to inspect pinned file: {error}"))?;
    file_identity(path, &mut file, &metadata, max_bytes)
}

#[cfg(not(unix))]
pub(super) fn pin_regular_file(_path: &Path, _max_bytes: usize) -> Result<FileIdentity, String> {
    Err("secure Git file pinning is unavailable on this platform".to_string())
}

#[cfg(unix)]
pub(super) fn open_verified_file(
    identity: &FileIdentity,
    max_bytes: usize,
) -> Result<fs::File, String> {
    let mut file = open_no_follow(Path::new(&identity.path))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("failed to inspect Git artifact: {error}"))?;
    let observed = file_identity(Path::new(&identity.path), &mut file, &metadata, max_bytes)?;
    if &observed != identity {
        return Err("Git artifact identity changed after it was frozen".to_string());
    }
    file.rewind()
        .map_err(|error| format!("failed to rewind Git artifact: {error}"))?;
    Ok(file)
}

#[cfg(unix)]
pub(super) fn verify_regular_file(identity: &FileIdentity, max_bytes: usize) -> Result<(), String> {
    let _ = open_verified_file(identity, max_bytes)?;
    Ok(())
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> Result<fs::File, String> {
    use std::os::unix::fs::OpenOptionsExt as _;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|error| format!("failed to securely open Git artifact: {error}"))
}

#[cfg(unix)]
fn file_identity(
    path: &Path,
    file: &mut fs::File,
    metadata: &fs::Metadata,
    max_bytes: usize,
) -> Result<FileIdentity, String> {
    use std::os::unix::fs::MetadataExt as _;

    if !metadata.is_file() || metadata.len() > max_bytes as u64 {
        return Err("Git artifact is not a bounded regular file".to_string());
    }
    file.rewind()
        .map_err(|error| format!("failed to rewind Git artifact: {error}"))?;
    let mut hasher = Sha256::new();
    let mut total = 0_usize;
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        let count = file
            .read(&mut chunk)
            .map_err(|error| format!("failed to hash Git artifact: {error}"))?;
        if count == 0 {
            break;
        }
        total = total.saturating_add(count);
        if total > max_bytes {
            return Err("Git artifact exceeded its read limit".to_string());
        }
        hasher.update(&chunk[..count]);
    }
    let after = file
        .metadata()
        .map_err(|error| format!("failed to recheck Git artifact: {error}"))?;
    if after.dev() != metadata.dev()
        || after.ino() != metadata.ino()
        || after.len() != metadata.len()
        || after.mode() != metadata.mode()
        || after.nlink() != metadata.nlink()
    {
        return Err("Git artifact changed while it was hashed".to_string());
    }
    Ok(FileIdentity {
        path: path
            .to_str()
            .ok_or_else(|| "Git artifact path is not UTF-8".to_string())?
            .to_string(),
        device: metadata.dev(),
        inode: metadata.ino(),
        owner: metadata.uid(),
        mode: metadata.mode(),
        link_count: metadata.nlink(),
        size: metadata.len(),
        digest: hex::encode(hasher.finalize()),
    })
}
