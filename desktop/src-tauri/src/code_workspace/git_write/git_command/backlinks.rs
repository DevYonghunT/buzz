use std::io::Read as _;
use std::path::{Path, PathBuf};

use super::identity::{open_verified_file, pin_regular_file};
use super::{DirectoryIdentity, FileIdentity, RepositoryAuthority};

const MAX_GIT_PATH_FILE_BYTES: usize = 32 * 1024;

pub(super) fn pin_admin_backlinks(
    root: &DirectoryIdentity,
    worktree_git_file: &FileIdentity,
    admin: &DirectoryIdentity,
    common: &DirectoryIdentity,
) -> Result<(FileIdentity, FileIdentity), String> {
    let admin_path = Path::new(&admin.path);
    let gitdir = pin_regular_file(&admin_path.join("gitdir"), MAX_GIT_PATH_FILE_BYTES)?;
    let commondir = pin_regular_file(&admin_path.join("commondir"), MAX_GIT_PATH_FILE_BYTES)?;
    validate_backlink_identity(&gitdir, &admin_path.join("gitdir"), admin.owner, "gitdir")?;
    validate_backlink_identity(
        &commondir,
        &admin_path.join("commondir"),
        admin.owner,
        "commondir",
    )?;
    verify_reciprocal_paths(root, worktree_git_file, admin, common, &gitdir, &commondir)?;
    Ok((gitdir, commondir))
}

pub(super) fn verify_admin_backlinks(
    root: &DirectoryIdentity,
    authority: &RepositoryAuthority,
) -> Result<(), String> {
    validate_backlink_identity(
        &authority.admin_gitdir_file,
        &Path::new(&authority.admin.path).join("gitdir"),
        authority.admin.owner,
        "gitdir",
    )?;
    validate_backlink_identity(
        &authority.admin_commondir_file,
        &Path::new(&authority.admin.path).join("commondir"),
        authority.admin.owner,
        "commondir",
    )?;
    verify_reciprocal_paths(
        root,
        &authority.worktree_git_file,
        &authority.admin,
        &authority.common,
        &authority.admin_gitdir_file,
        &authority.admin_commondir_file,
    )
}

fn verify_reciprocal_paths(
    root: &DirectoryIdentity,
    worktree_git_file: &FileIdentity,
    admin: &DirectoryIdentity,
    common: &DirectoryIdentity,
    admin_gitdir_file: &FileIdentity,
    admin_commondir_file: &FileIdentity,
) -> Result<(), String> {
    let root_path = Path::new(&root.path);
    let admin_path = Path::new(&admin.path);
    let common_path = Path::new(&common.path);
    if admin_path.parent() != Some(common_path.join("worktrees").as_path()) {
        return Err("Git admin directory escaped the pinned common-dir/worktrees boundary".into());
    }

    let root_git_target = parse_path_file(
        &read_verified(worktree_git_file)?,
        Some(b"gitdir: "),
        root_path,
        "linked-worktree .git",
    )?;
    if canonical_path(&root_git_target, "linked-worktree Git admin")? != admin_path {
        return Err("linked-worktree .git does not point to the pinned Git admin".to_string());
    }

    let admin_gitdir = parse_path_file(
        &read_verified(admin_gitdir_file)?,
        None,
        admin_path,
        "Git-admin gitdir",
    )?;
    let expected_root_git =
        canonical_path(Path::new(&worktree_git_file.path), "pinned worktree .git")?;
    if canonical_path(&admin_gitdir, "Git-admin worktree backlink")? != expected_root_git {
        return Err("Git-admin gitdir does not point back to the pinned worktree .git".to_string());
    }

    let admin_commondir = parse_path_file(
        &read_verified(admin_commondir_file)?,
        None,
        admin_path,
        "Git-admin commondir",
    )?;
    if canonical_path(&admin_commondir, "Git-admin common-dir backlink")? != common_path {
        return Err(
            "Git-admin commondir does not point to the pinned common directory".to_string(),
        );
    }
    Ok(())
}

fn validate_backlink_identity(
    identity: &FileIdentity,
    expected_path: &Path,
    expected_owner: u32,
    label: &str,
) -> Result<(), String> {
    if Path::new(&identity.path) != expected_path
        || identity.owner != expected_owner
        || identity.link_count != 1
        || identity.mode & 0o170000 != 0o100000
    {
        return Err(format!(
            "Git-admin {label} is not an exact singly-linked owner file"
        ));
    }
    Ok(())
}

fn read_verified(identity: &FileIdentity) -> Result<Vec<u8>, String> {
    let mut file = open_verified_file(identity, MAX_GIT_PATH_FILE_BYTES)?;
    let mut bytes = Vec::with_capacity(identity.size as usize);
    file.read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read pinned Git backlink: {error}"))?;
    if bytes.len() as u64 != identity.size {
        return Err("Pinned Git backlink size changed while it was read".to_string());
    }
    Ok(bytes)
}

fn parse_path_file(
    bytes: &[u8],
    prefix: Option<&[u8]>,
    relative_to: &Path,
    label: &str,
) -> Result<PathBuf, String> {
    let bytes = match prefix {
        Some(prefix) => bytes
            .strip_prefix(prefix)
            .ok_or_else(|| format!("{label} has an invalid prefix"))?,
        None => bytes,
    };
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
    let value = std::str::from_utf8(value).map_err(|_| format!("{label} is not UTF-8"))?;
    let path = PathBuf::from(value);
    Ok(if path.is_absolute() {
        path
    } else {
        relative_to.join(path)
    })
}

fn canonical_path(path: &Path, label: &str) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|error| format!("failed to resolve {label}: {error}"))
}
