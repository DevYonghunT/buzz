use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::git_command::{pin_directory, pin_input_file, DirectoryIdentity, FileIdentity};

const CODE_DIRECTORY: &str = "code";
const ARTIFACT_DIRECTORY: &str = "git-candidates";
const MAX_PRIVATE_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;

pub(super) struct PrivateArtifactStore {
    directory: PathBuf,
    identity: DirectoryIdentity,
}

impl PrivateArtifactStore {
    pub(super) fn for_mutation(app_data_dir: &Path) -> Result<Self, String> {
        #[cfg(not(unix))]
        {
            let _ = app_data_dir;
            return Err(
                "SchoolX Code Git writes require private Unix artifact support".to_string(),
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt as _, MetadataExt as _};

            validate_real_directory(app_data_dir, "app-data")?;
            let app_data_dir = app_data_dir
                .canonicalize()
                .map_err(|error| format!("failed to resolve app-data directory: {error}"))?;
            let app_owner = fs::symlink_metadata(&app_data_dir)
                .map_err(|error| format!("failed to inspect app-data owner: {error}"))?
                .uid();
            let code_dir = app_data_dir.join(CODE_DIRECTORY);
            ensure_private_directory(&code_dir, app_owner, true)?;
            let directory = code_dir.join(ARTIFACT_DIRECTORY);
            let existed = fs::symlink_metadata(&directory).is_ok();
            if !existed {
                let mut builder = fs::DirBuilder::new();
                builder.mode(0o700);
                builder.create(&directory).map_err(|error| {
                    format!("failed to create private Git artifact directory: {error}")
                })?;
                sync_directory(&code_dir)?;
            }
            ensure_private_directory(&directory, app_owner, false)?;
            let directory = directory.canonicalize().map_err(|error| {
                format!("failed to resolve private Git artifact directory: {error}")
            })?;
            if directory.parent() != Some(code_dir.as_path()) {
                return Err("private Git artifact directory escaped app data".to_string());
            }
            let identity = pin_directory(&directory)?;
            Ok(Self {
                directory,
                identity,
            })
        }
    }

    pub(super) fn create(&self, label: &str, bytes: &[u8]) -> Result<FileIdentity, String> {
        #[cfg(not(unix))]
        {
            let _ = (label, bytes);
            return Err(
                "SchoolX Code Git writes require private Unix artifact support".to_string(),
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

            self.revalidate()?;
            validate_label(label)?;
            if bytes.len() > MAX_PRIVATE_ARTIFACT_BYTES {
                return Err(format!(
                    "private Git artifact exceeds {MAX_PRIVATE_ARTIFACT_BYTES} bytes"
                ));
            }
            let name = format!("{}-{}", label, random_hex()?);
            let path = self.directory.join(name);
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
                .open(&path)
                .map_err(|error| format!("failed to create private Git artifact: {error}"))?;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| format!("failed to secure private Git artifact: {error}"))?;
            file.write_all(bytes)
                .map_err(|error| format!("failed to write private Git artifact: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("failed to sync private Git artifact: {error}"))?;
            sync_directory(&self.directory)?;
            self.revalidate()?;
            let evidence = pin_input_file(&path, MAX_PRIVATE_ARTIFACT_BYTES)?;
            if evidence.mode & 0o7777 != 0o600 || evidence.link_count != 1 {
                return Err("private Git artifact was not a single owner-only file".to_string());
            }
            Ok(evidence)
        }
    }

    pub(super) fn refresh(&self, path: &Path) -> Result<FileIdentity, String> {
        self.revalidate()?;
        self.require_child(path)?;
        let evidence = pin_input_file(path, MAX_PRIVATE_ARTIFACT_BYTES)?;
        if evidence.mode & 0o7777 != 0o600 || evidence.link_count != 1 {
            return Err("private Git artifact permission or link identity changed".to_string());
        }
        Ok(evidence)
    }

    /// Re-secure a private file after a sealed Git command intentionally
    /// replaced it (for example `update-index` publishing its own lock file).
    pub(super) fn secure_after_mutation(&self, path: &Path) -> Result<FileIdentity, String> {
        #[cfg(not(unix))]
        {
            let _ = path;
            return Err(
                "SchoolX Code Git writes require private Unix artifact support".to_string(),
            );
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};

            self.revalidate()?;
            self.require_child(path)?;
            let file = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path)
                .map_err(|error| format!("failed to reopen mutated private artifact: {error}"))?;
            let before = file.metadata().map_err(|error| {
                format!("failed to inspect mutated private Git artifact: {error}")
            })?;
            if !before.is_file()
                || before.uid() != self.identity.owner
                || before.nlink() != 1
                || before.len() > MAX_PRIVATE_ARTIFACT_BYTES as u64
            {
                return Err(
                    "mutated private Git artifact is not a singly-linked owner file".to_string(),
                );
            }
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| format!("failed to re-secure private Git artifact: {error}"))?;
            file.sync_all()
                .map_err(|error| format!("failed to sync re-secured Git artifact: {error}"))?;
            sync_directory(&self.directory)?;
            let opened = file.metadata().map_err(|error| {
                format!("failed to re-inspect mutated private Git artifact: {error}")
            })?;
            let evidence = pin_input_file(path, MAX_PRIVATE_ARTIFACT_BYTES)?;
            if evidence.device != opened.dev()
                || evidence.inode != opened.ino()
                || evidence.owner != opened.uid()
                || evidence.size != opened.len()
                || evidence.mode & 0o7777 != 0o600
                || evidence.link_count != 1
            {
                return Err("mutated private Git artifact changed while re-securing".to_string());
            }
            self.revalidate()?;
            Ok(evidence)
        }
    }

    pub(super) fn read(&self, expected: &FileIdentity) -> Result<Vec<u8>, String> {
        self.revalidate()?;
        let path = Path::new(&expected.path);
        self.require_child(path)?;
        let observed = self.refresh(path)?;
        if &observed != expected {
            return Err("private Git artifact changed after it was journaled".to_string());
        }
        read_bounded_file(path, MAX_PRIVATE_ARTIFACT_BYTES)
    }

    pub(super) fn remove(&self, expected: &FileIdentity) -> Result<(), String> {
        self.revalidate()?;
        let path = Path::new(&expected.path);
        self.require_child(path)?;
        let observed = self.refresh(path)?;
        if &observed != expected {
            return Err("refusing to remove a replaced private Git artifact".to_string());
        }
        fs::remove_file(path)
            .map_err(|error| format!("failed to remove private Git artifact: {error}"))?;
        sync_directory(&self.directory)?;
        self.revalidate()?;
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Ok(_) => Err("private Git artifact remained after cleanup".to_string()),
            Err(error) => Err(format!(
                "failed to verify private artifact cleanup: {error}"
            )),
        }
    }

    /// Converge cleanup when an exact private artifact may already be absent.
    pub(super) fn remove_if_absent_or_exact(&self, expected: &FileIdentity) -> Result<(), String> {
        self.revalidate()?;
        let path = Path::new(&expected.path);
        self.require_child(path)?;
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                sync_directory(&self.directory)?;
                return self.revalidate();
            }
            Err(error) => {
                return Err(format!(
                    "failed to inspect private Git artifact cleanup: {error}"
                ));
            }
            Ok(_) => {}
        }
        match self.remove(expected) {
            Ok(()) => Ok(()),
            Err(error) => match fs::symlink_metadata(path) {
                Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => {
                    sync_directory(&self.directory)?;
                    self.revalidate()
                }
                _ => Err(error),
            },
        }
    }

    fn revalidate(&self) -> Result<(), String> {
        let current = pin_directory(&self.directory)?;
        // A directory's link count is not a stable replacement signal on every
        // supported Unix filesystem: creating or unlinking children may change
        // it.  Device/inode plus owner and mode keep this check descriptor-bound
        // without rejecting our own durable artifact mutations.
        if current.path != self.identity.path
            || current.device != self.identity.device
            || current.inode != self.identity.inode
            || current.owner != self.identity.owner
            || current.mode != self.identity.mode
        {
            return Err("private Git artifact directory moved or was replaced".to_string());
        }
        Ok(())
    }

    fn require_child(&self, path: &Path) -> Result<(), String> {
        if path.parent() != Some(self.directory.as_path()) {
            return Err("private Git artifact escaped its pinned directory".to_string());
        }
        Ok(())
    }
}

fn validate_label(label: &str) -> Result<(), String> {
    if label.is_empty()
        || label.len() > 64
        || !label
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err("private Git artifact label is invalid".to_string());
    }
    Ok(())
}

fn random_hex() -> Result<String, String> {
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| format!("failed to create private Git artifact name: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn read_bounded_file(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    #[cfg(unix)]
    use std::os::unix::fs::OpenOptionsExt as _;

    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let file = options
        .open(path)
        .map_err(|error| format!("failed to open private Git artifact: {error}"))?;
    let metadata = file
        .metadata()
        .map_err(|error| format!("failed to inspect private Git artifact: {error}"))?;
    if !metadata.is_file() || metadata.len() > limit as u64 {
        return Err("private Git artifact is not a bounded regular file".to_string());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read private Git artifact: {error}"))?;
    if bytes.len() > limit {
        return Err("private Git artifact exceeded its read limit".to_string());
    }
    Ok(bytes)
}

fn validate_real_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect {label} directory: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!("{label} path is not a real directory"));
    }
    Ok(())
}

#[cfg(unix)]
fn ensure_private_directory(path: &Path, owner: u32, allow_existing: bool) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect private Git directory: {error}"))?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != owner
        || metadata.mode() & 0o7777 != 0o700
    {
        return Err("private Git directory is not an owner-only real directory".to_string());
    }
    let _ = allow_existing;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), String> {
    fs::File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|error| format!("failed to sync private Git directory: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn private_artifacts_are_durable_private_and_replacement_safe() -> Result<(), String> {
        use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};

        let app_data = tempfile::tempdir().map_err(|error| error.to_string())?;
        fs::set_permissions(app_data.path(), fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder
            .create(app_data.path().join("code"))
            .map_err(|error| error.to_string())?;
        let store = PrivateArtifactStore::for_mutation(app_data.path())
            .map_err(|error| format!("store setup: {error}"))?;
        let evidence = store
            .create("index", b"candidate\n")
            .map_err(|error| format!("artifact create: {error}"))?;
        assert_eq!(store.read(&evidence)?, b"candidate\n");
        fs::write(&evidence.path, b"replacement\n").map_err(|error| error.to_string())?;
        let error = store
            .remove(&evidence)
            .expect_err("replacement must not be removed");
        assert!(error.contains("replaced") || error.contains("changed"));
        assert_eq!(
            fs::read(&evidence.path).map_err(|error| error.to_string())?,
            b"replacement\n"
        );
        Ok(())
    }
}
