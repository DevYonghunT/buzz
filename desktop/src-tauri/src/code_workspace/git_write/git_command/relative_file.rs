use std::fs;
use std::io::Read;

#[derive(Clone, Debug)]
pub(in super::super) struct FrozenWorktreeFile {
    pub(in super::super) bytes: Vec<u8>,
    pub(in super::super) executable: bool,
}

#[cfg(unix)]
pub(super) fn read_relative(
    root: &fs::File,
    relative: &str,
    max_bytes: usize,
) -> Result<Option<FrozenWorktreeFile>, String> {
    use std::ffi::CString;
    use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
    use std::os::unix::fs::MetadataExt as _;

    super::validate_relative_path(relative)?;
    let parts = relative.split('/').collect::<Vec<_>>();
    let mut directory: Option<OwnedFd> = None;
    for part in &parts[..parts.len().saturating_sub(1)] {
        let part = CString::new(*part)
            .map_err(|_| "Git path component contained a NUL byte".to_string())?;
        let parent: BorrowedFd<'_> = directory
            .as_ref()
            .map(AsFd::as_fd)
            .unwrap_or_else(|| root.as_fd());
        let opened = rustix::fs::openat(
            parent,
            part.as_c_str(),
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|error| format!("failed to pin Git worktree path component: {error}"))?;
        directory = Some(opened);
    }
    let leaf = CString::new(
        *parts
            .last()
            .ok_or_else(|| "Git path was empty".to_string())?,
    )
    .map_err(|_| "Git path contained a NUL byte".to_string())?;
    let parent: BorrowedFd<'_> = directory
        .as_ref()
        .map(AsFd::as_fd)
        .unwrap_or_else(|| root.as_fd());
    let opened = match rustix::fs::openat(
        parent,
        leaf.as_c_str(),
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC
            | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    ) {
        Ok(file) => file,
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(format!("failed to freeze Git worktree file: {error}")),
    };
    let mut file = fs::File::from(opened);
    let before = file
        .metadata()
        .map_err(|error| format!("failed to inspect frozen Git worktree file: {error}"))?;
    if !before.is_file() || before.len() > max_bytes as u64 {
        return Err("whole-file Git write requires a bounded regular file".to_string());
    }
    let mut bytes = Vec::with_capacity(before.len() as usize);
    (&mut file)
        .take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read frozen Git worktree file: {error}"))?;
    if bytes.len() > max_bytes {
        return Err("whole-file Git write source exceeded its limit".to_string());
    }
    let after = file
        .metadata()
        .map_err(|error| format!("failed to recheck frozen Git worktree file: {error}"))?;
    if before.dev() != after.dev()
        || before.ino() != after.ino()
        || before.mode() != after.mode()
        || before.len() != after.len()
    {
        return Err("Git worktree file changed while it was frozen".to_string());
    }
    Ok(Some(FrozenWorktreeFile {
        bytes,
        executable: before.mode() & 0o111 != 0,
    }))
}

#[cfg(not(unix))]
pub(super) fn read_relative(
    _root: &fs::File,
    _relative: &str,
    _max_bytes: usize,
) -> Result<Option<FrozenWorktreeFile>, String> {
    Err("descriptor-bound worktree reads are unavailable on this platform".to_string())
}
