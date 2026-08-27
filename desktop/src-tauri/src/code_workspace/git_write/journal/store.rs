use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::{
    random_hex, GitOperationJournal, JOURNAL_DIRECTORY, JOURNAL_FILE,
    MAX_GIT_OPERATION_JOURNAL_BYTES,
};

/// Filesystem-backed strict journal store. Reads never create or chmod paths.
#[derive(Clone, Debug)]
pub(crate) struct GitOperationJournalStore {
    app_data_dir: PathBuf,
    code_dir: PathBuf,
    journal_path: PathBuf,
}

impl GitOperationJournalStore {
    pub(crate) fn for_app_data(app_data_dir: &Path) -> Result<Self, String> {
        if !app_data_dir.is_absolute() {
            return Err("SchoolX Code Git journal app-data path must be absolute".to_string());
        }
        let metadata = fs::symlink_metadata(app_data_dir).map_err(|error| {
            format!("failed to inspect Git journal app-data directory: {error}")
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(
                "SchoolX Code Git journal app-data path must be a real directory".to_string(),
            );
        }
        let app_data_dir = app_data_dir.canonicalize().map_err(|error| {
            format!("failed to resolve Git journal app-data directory: {error}")
        })?;
        let code_dir = app_data_dir.join(JOURNAL_DIRECTORY);
        Ok(Self {
            journal_path: code_dir.join(JOURNAL_FILE),
            app_data_dir,
            code_dir,
        })
    }

    /// Strictly load the journal. Missing `code` or journal paths represent an
    /// empty store and do not create, chmod, rename, or fsync anything.
    pub(crate) fn load(&self) -> Result<GitOperationJournal, String> {
        #[cfg(unix)]
        {
            let Some(directory) = self.open_code_directory(false)? else {
                return Ok(GitOperationJournal::default());
            };
            self.load_from_directory(&directory)
                .map(|(journal, _)| journal)
        }
        #[cfg(not(unix))]
        {
            // Strict Git writes and managed worktrees remain unavailable on
            // this platform. Local-mode Code tasks do not create this journal,
            // so an absent file is the only safe state needed at startup.
            // Any existing bytes still fail closed because this build cannot
            // validate or recover their Unix-specific ownership evidence.
            load_unsupported_platform_empty_journal(
                &self.app_data_dir,
                &self.code_dir,
                &self.journal_path,
            )
        }
    }

    /// Compact acknowledged history, validate the complete next image, and
    /// publish it through an owner-only temp file in the pinned parent.
    pub(crate) fn save(&self, journal: &GitOperationJournal) -> Result<(), String> {
        journal.validate_for_compaction()?;
        let mut journal = journal.clone();
        journal.compact_acknowledged();
        journal.validate()?;
        let mut payload = serde_json::to_vec_pretty(&journal)
            .map_err(|error| format!("failed to encode Git operation journal: {error}"))?;
        payload.push(b'\n');
        if payload.len() as u64 > MAX_GIT_OPERATION_JOURNAL_BYTES {
            return Err(format!(
                "SchoolX Code Git journal exceeds the {MAX_GIT_OPERATION_JOURNAL_BYTES}-byte limit"
            ));
        }

        let directory = self
            .open_code_directory(true)?
            .ok_or_else(|| "Git journal directory was not created".to_string())?;
        // Refuse to overwrite malformed, oversized, or weakly permissioned
        // existing bytes. The returned identity is the CAS preimage below.
        let (_, previous) = self.load_from_directory(&directory)?;
        let temp_name = format!(".git-operations-{}.tmp", random_hex()?);
        let (mut temp_file, temp_identity) = create_private_temp(&directory, &temp_name)?;
        let mut temp = OwnedTemp::new(&directory, temp_name, temp_identity.clone());
        temp_file
            .write_all(&payload)
            .map_err(|error| format!("failed to write Git operation journal: {error}"))?;
        temp_file
            .sync_all()
            .map_err(|error| format!("failed to sync Git operation journal: {error}"))?;
        let synced_identity = file_identity(&temp_file)?;
        validate_private_file_identity(&synced_identity, Some(payload.len() as u64))?;
        if !temp_identity.same_object(&synced_identity)
            || current_entry_identity(&directory, temp.name())? != Some(synced_identity.clone())
        {
            return Err("Git journal temp file was replaced before publish".to_string());
        }
        ensure_named_directory_identity(&self.code_dir, &directory.identity)?;
        if current_entry_identity(&directory, JOURNAL_FILE)? != previous {
            return Err("Git operation journal changed before atomic publish".to_string());
        }
        rename_entry(&directory, temp.name(), JOURNAL_FILE)?;
        temp.mark_published();
        let published = current_entry_identity(&directory, JOURNAL_FILE)?
            .ok_or_else(|| "Published Git operation journal disappeared".to_string())?;
        if !published.same_object(&synced_identity) {
            return Err("Published Git operation journal has the wrong identity".to_string());
        }
        ensure_named_directory_identity(&self.code_dir, &directory.identity)?;
        directory
            .file
            .sync_all()
            .map_err(|error| format!("failed to sync Git journal directory: {error}"))
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        &self.journal_path
    }

    fn load_from_directory(
        &self,
        directory: &DirectoryHandle,
    ) -> Result<(GitOperationJournal, Option<FsIdentity>), String> {
        ensure_named_directory_identity(&self.code_dir, &directory.identity)?;
        let Some(file) = open_existing_entry(directory, JOURNAL_FILE)? else {
            return Ok((GitOperationJournal::default(), None));
        };
        let identity = file_identity(&file)?;
        validate_private_file_identity(&identity, None)?;
        if identity.size > MAX_GIT_OPERATION_JOURNAL_BYTES {
            return Err(format!(
                "SchoolX Code Git journal exceeds the {MAX_GIT_OPERATION_JOURNAL_BYTES}-byte limit"
            ));
        }
        if current_entry_identity(directory, JOURNAL_FILE)? != Some(identity.clone()) {
            return Err("Git operation journal was replaced during strict load".to_string());
        }
        ensure_named_directory_identity(&self.code_dir, &directory.identity)?;

        let mut bytes = Vec::new();
        (&file)
            .take(MAX_GIT_OPERATION_JOURNAL_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("failed to read Git operation journal: {error}"))?;
        if bytes.len() as u64 > MAX_GIT_OPERATION_JOURNAL_BYTES {
            return Err(format!(
                "SchoolX Code Git journal exceeds the {MAX_GIT_OPERATION_JOURNAL_BYTES}-byte limit"
            ));
        }
        let after = file_identity(&file)?;
        if after != identity
            || current_entry_identity(directory, JOURNAL_FILE)? != Some(identity.clone())
        {
            return Err("Git operation journal changed during strict load".to_string());
        }
        ensure_named_directory_identity(&self.code_dir, &directory.identity)?;
        let journal: GitOperationJournal = serde_json::from_slice(&bytes)
            .map_err(|error| format!("failed to decode Git operation journal: {error}"))?;
        journal.validate()?;
        Ok((journal, Some(identity)))
    }

    fn open_code_directory(&self, create: bool) -> Result<Option<DirectoryHandle>, String> {
        match fs::symlink_metadata(&self.code_dir) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(
                        "SchoolX Code Git journal directory must be a real directory".to_string(),
                    );
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !create => {
                return Ok(None);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                create_private_directory(&self.app_data_dir, &self.code_dir)?;
            }
            Err(error) => {
                return Err(format!("failed to inspect Git journal directory: {error}"));
            }
        }
        open_private_directory(&self.code_dir).map(Some)
    }
}

#[cfg(any(not(unix), test))]
fn load_unsupported_platform_empty_journal(
    app_data_dir: &Path,
    code_dir: &Path,
    journal_path: &Path,
) -> Result<GitOperationJournal, String> {
    if journal_path.parent() != Some(code_dir) {
        return Err("SchoolX Code Git journal escaped its data directory".to_string());
    }
    let metadata = match fs::symlink_metadata(code_dir) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(GitOperationJournal::default());
        }
        Err(error) => {
            return Err(format!(
                "failed to inspect SchoolX Code Git journal directory: {error}"
            ));
        }
    };
    if is_symlink_or_reparse_point(&metadata) || !metadata.is_dir() {
        return Err("SchoolX Code Git journal directory must be a real directory".to_string());
    }
    let resolved_code_dir = code_dir.canonicalize().map_err(|error| {
        format!("failed to resolve SchoolX Code Git journal directory: {error}")
    })?;
    if resolved_code_dir.parent() != Some(app_data_dir)
        || !resolved_code_dir.starts_with(app_data_dir)
    {
        return Err("SchoolX Code Git journal directory escaped the app-data root".to_string());
    }

    match fs::symlink_metadata(journal_path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let after = code_dir.canonicalize().map_err(|resolve_error| {
                format!("failed to revalidate SchoolX Code Git journal directory: {resolve_error}")
            })?;
            if after != resolved_code_dir {
                return Err("SchoolX Code Git journal directory changed during startup".to_string());
            }
            Ok(GitOperationJournal::default())
        }
        Ok(_) => Err(
            "SchoolX Code found a Git journal that this platform cannot safely recover".to_string(),
        ),
        Err(error) => Err(format!(
            "failed to inspect SchoolX Code Git journal on this platform: {error}"
        )),
    }
}

#[cfg(any(not(unix), test))]
fn is_symlink_or_reparse_point(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    false
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FsIdentity {
    device: u64,
    inode: u64,
    owner: u32,
    mode: u32,
    link_count: u64,
    size: u64,
}

impl FsIdentity {
    fn same_object(&self, other: &Self) -> bool {
        self.device == other.device
            && self.inode == other.inode
            && self.owner == other.owner
            && self.mode == other.mode
            && self.link_count == other.link_count
    }

    fn same_directory(&self, other: &Self) -> bool {
        self.device == other.device
            && self.inode == other.inode
            && self.owner == other.owner
            && self.mode == other.mode
    }
}

struct DirectoryHandle {
    file: File,
    identity: FsIdentity,
}

struct OwnedTemp<'a> {
    directory: &'a DirectoryHandle,
    name: String,
    identity: FsIdentity,
    published: bool,
}

impl<'a> OwnedTemp<'a> {
    fn new(directory: &'a DirectoryHandle, name: String, identity: FsIdentity) -> Self {
        Self {
            directory,
            name,
            identity,
            published: false,
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn mark_published(&mut self) {
        self.published = true;
    }
}

impl Drop for OwnedTemp<'_> {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        if current_entry_identity(self.directory, &self.name)
            .ok()
            .flatten()
            .is_some_and(|identity| identity.same_object(&self.identity))
        {
            let _ = unlink_entry(self.directory, &self.name);
            let _ = self.directory.file.sync_all();
        }
    }
}

#[cfg(unix)]
fn create_private_directory(app_data_dir: &Path, code_dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::DirBuilderExt as _;

    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);
    match builder.create(code_dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(format!("failed to create Git journal directory: {error}")),
    }
    let parent = open_directory_unchecked(app_data_dir, "Git journal app-data directory")?;
    parent
        .sync_all()
        .map_err(|error| format!("failed to sync Git journal app-data directory: {error}"))
}

#[cfg(not(unix))]
fn create_private_directory(_app_data_dir: &Path, _code_dir: &Path) -> Result<(), String> {
    Err("Strict SchoolX Code Git journal storage is unsupported on this platform".to_string())
}

#[cfg(unix)]
fn open_private_directory(path: &Path) -> Result<DirectoryHandle, String> {
    let file = open_directory_unchecked(path, "Git journal directory")?;
    let identity = file_identity(&file)?;
    validate_directory_identity(&identity)?;
    ensure_named_directory_identity(path, &identity)?;
    Ok(DirectoryHandle { file, identity })
}

#[cfg(not(unix))]
fn open_private_directory(_path: &Path) -> Result<DirectoryHandle, String> {
    Err("Strict SchoolX Code Git journal storage is unsupported on this platform".to_string())
}

#[cfg(unix)]
fn open_directory_unchecked(path: &Path, label: &str) -> Result<File, String> {
    use std::os::unix::fs::OpenOptionsExt as _;

    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| format!("failed to open {label}: {error}"))
}

#[cfg(unix)]
fn open_existing_entry(directory: &DirectoryHandle, name: &str) -> Result<Option<File>, String> {
    use rustix::fs::{Mode, OFlags};

    match rustix::fs::openat(
        &directory.file,
        name,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(file) => Ok(Some(File::from(file))),
        Err(error) if error == rustix::io::Errno::NOENT => Ok(None),
        Err(error) => Err(format!("failed to open Git operation journal: {error}")),
    }
}

#[cfg(not(unix))]
fn open_existing_entry(_directory: &DirectoryHandle, _name: &str) -> Result<Option<File>, String> {
    Err("Strict SchoolX Code Git journal storage is unsupported on this platform".to_string())
}

#[cfg(unix)]
fn create_private_temp(
    directory: &DirectoryHandle,
    name: &str,
) -> Result<(File, FsIdentity), String> {
    use rustix::fs::{Mode, OFlags};

    let descriptor = rustix::fs::openat(
        &directory.file,
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|error| format!("failed to create Git journal temp file: {error}"))?;
    let file = File::from(descriptor);
    let identity = file_identity(&file)?;
    validate_private_file_identity(&identity, Some(0))?;
    Ok((file, identity))
}

#[cfg(not(unix))]
fn create_private_temp(
    _directory: &DirectoryHandle,
    _name: &str,
) -> Result<(File, FsIdentity), String> {
    Err("Strict SchoolX Code Git journal storage is unsupported on this platform".to_string())
}

#[cfg(unix)]
fn rename_entry(directory: &DirectoryHandle, from: &str, to: &str) -> Result<(), String> {
    rustix::fs::renameat(&directory.file, from, &directory.file, to)
        .map_err(|error| format!("failed to publish Git operation journal: {error}"))
}

#[cfg(not(unix))]
fn rename_entry(_directory: &DirectoryHandle, _from: &str, _to: &str) -> Result<(), String> {
    Err("Strict SchoolX Code Git journal storage is unsupported on this platform".to_string())
}

#[cfg(unix)]
fn unlink_entry(directory: &DirectoryHandle, name: &str) -> Result<(), String> {
    rustix::fs::unlinkat(&directory.file, name, rustix::fs::AtFlags::empty())
        .map_err(|error| format!("failed to remove owned Git journal temp file: {error}"))
}

#[cfg(not(unix))]
fn unlink_entry(_directory: &DirectoryHandle, _name: &str) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn file_identity(file: &File) -> Result<FsIdentity, String> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = file
        .metadata()
        .map_err(|error| format!("failed to inspect Git journal filesystem object: {error}"))?;
    Ok(FsIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        owner: metadata.uid(),
        mode: metadata.mode(),
        link_count: metadata.nlink(),
        size: metadata.size(),
    })
}

#[cfg(not(unix))]
fn file_identity(_file: &File) -> Result<FsIdentity, String> {
    Err("Strict SchoolX Code Git journal storage is unsupported on this platform".to_string())
}

#[cfg(unix)]
fn current_entry_identity(
    directory: &DirectoryHandle,
    name: &str,
) -> Result<Option<FsIdentity>, String> {
    open_existing_entry(directory, name)?
        .map(|file| file_identity(&file))
        .transpose()
}

#[cfg(not(unix))]
fn current_entry_identity(
    _directory: &DirectoryHandle,
    _name: &str,
) -> Result<Option<FsIdentity>, String> {
    Err("Strict SchoolX Code Git journal storage is unsupported on this platform".to_string())
}

#[cfg(unix)]
fn ensure_named_directory_identity(path: &Path, expected: &FsIdentity) -> Result<(), String> {
    let current = open_directory_unchecked(path, "named Git journal directory")?;
    let current = file_identity(&current)?;
    validate_directory_identity(&current)?;
    if !current.same_directory(expected) {
        return Err("Git journal directory was replaced during the operation".to_string());
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_named_directory_identity(_path: &Path, _expected: &FsIdentity) -> Result<(), String> {
    Err("Strict SchoolX Code Git journal storage is unsupported on this platform".to_string())
}

#[cfg(unix)]
fn validate_directory_identity(identity: &FsIdentity) -> Result<(), String> {
    let current_uid = rustix::process::geteuid().as_raw();
    if identity.mode & u32::from(libc::S_IFMT) != u32::from(libc::S_IFDIR)
        || identity.owner != current_uid
        || identity.mode & 0o7777 != 0o700
    {
        return Err("SchoolX Code Git journal directory must be owner-owned mode 0700".to_string());
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_directory_identity(_identity: &FsIdentity) -> Result<(), String> {
    Err("Strict SchoolX Code Git journal storage is unsupported on this platform".to_string())
}

#[cfg(unix)]
fn validate_private_file_identity(
    identity: &FsIdentity,
    expected_size: Option<u64>,
) -> Result<(), String> {
    let current_uid = rustix::process::geteuid().as_raw();
    if identity.mode & u32::from(libc::S_IFMT) != u32::from(libc::S_IFREG)
        || identity.owner != current_uid
        || identity.mode & 0o7777 != 0o600
        || identity.link_count != 1
    {
        return Err(
            "SchoolX Code Git journal file must be an owner-owned, singly-linked regular file with mode 0600"
                .to_string(),
        );
    }
    if expected_size.is_some_and(|size| size != identity.size) {
        return Err("SchoolX Code Git journal file has an unexpected size".to_string());
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_file_identity(
    _identity: &FsIdentity,
    _expected_size: Option<u64>,
) -> Result<(), String> {
    Err("Strict SchoolX Code Git journal storage is unsupported on this platform".to_string())
}

#[cfg(test)]
mod unsupported_platform_load_tests {
    use super::*;

    #[test]
    fn absent_journal_is_an_empty_zero_mutation_store() -> Result<(), String> {
        let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
        let app_data_dir = temp
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let code_dir = app_data_dir.join("code");
        let journal_path = code_dir.join(JOURNAL_FILE);

        let loaded =
            load_unsupported_platform_empty_journal(&app_data_dir, &code_dir, &journal_path)?;

        assert_eq!(loaded, GitOperationJournal::default());
        assert!(!code_dir.exists());
        Ok(())
    }

    #[test]
    fn existing_data_directory_with_no_journal_is_an_empty_store() -> Result<(), String> {
        let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
        let app_data_dir = temp
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let code_dir = app_data_dir.join("code");
        fs::create_dir(&code_dir).map_err(|error| error.to_string())?;
        let journal_path = code_dir.join(JOURNAL_FILE);

        let loaded =
            load_unsupported_platform_empty_journal(&app_data_dir, &code_dir, &journal_path)?;

        assert_eq!(loaded, GitOperationJournal::default());
        assert!(code_dir.is_dir());
        assert!(!journal_path.exists());
        Ok(())
    }

    #[test]
    fn existing_journal_fails_closed_without_changing_bytes() -> Result<(), String> {
        let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
        let app_data_dir = temp
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let code_dir = app_data_dir.join("code");
        fs::create_dir(&code_dir).map_err(|error| error.to_string())?;
        let journal_path = code_dir.join(JOURNAL_FILE);
        let bytes = b"preserve unsupported journal evidence";
        fs::write(&journal_path, bytes).map_err(|error| error.to_string())?;

        let error =
            load_unsupported_platform_empty_journal(&app_data_dir, &code_dir, &journal_path)
                .expect_err("an existing unsupported-platform journal must fail closed");

        assert!(error.contains("cannot safely recover"));
        assert_eq!(
            fs::read(&journal_path).map_err(|read_error| read_error.to_string())?,
            bytes
        );
        Ok(())
    }
}

#[cfg(all(test, windows))]
mod windows_store_tests {
    use super::*;

    fn windows_store() -> Result<(tempfile::TempDir, PathBuf, GitOperationJournalStore), String> {
        let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
        let app_data_dir = temp
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let store = GitOperationJournalStore::for_app_data(&app_data_dir)?;
        Ok((temp, app_data_dir, store))
    }

    #[test]
    fn windows_existing_code_directory_without_git_journal_loads_empty_without_mutation(
    ) -> Result<(), String> {
        let (_temp, app_data_dir, store) = windows_store()?;
        let code_dir = app_data_dir.join(JOURNAL_DIRECTORY);
        fs::create_dir(&code_dir).map_err(|error| error.to_string())?;
        let marker = code_dir.join("binding-store-marker");
        fs::write(&marker, b"preserve").map_err(|error| error.to_string())?;

        let loaded = store.load()?;

        assert_eq!(loaded, GitOperationJournal::default());
        assert!(!store.path().exists());
        assert_eq!(
            fs::read(&marker).map_err(|error| error.to_string())?,
            b"preserve"
        );
        Ok(())
    }

    #[test]
    fn windows_existing_git_journal_fails_closed_and_preserves_bytes() -> Result<(), String> {
        let (_temp, app_data_dir, store) = windows_store()?;
        let code_dir = app_data_dir.join(JOURNAL_DIRECTORY);
        fs::create_dir(&code_dir).map_err(|error| error.to_string())?;
        let bytes = b"unsupported durable evidence";
        fs::write(store.path(), bytes).map_err(|error| error.to_string())?;

        let error = store
            .load()
            .expect_err("Windows must not recover an existing Unix journal");

        assert!(error.contains("cannot safely recover"));
        assert_eq!(
            fs::read(store.path()).map_err(|read_error| read_error.to_string())?,
            bytes
        );
        Ok(())
    }

    #[test]
    fn windows_git_journal_save_remains_unsupported_without_creating_files() -> Result<(), String> {
        let (_temp, app_data_dir, store) = windows_store()?;

        let error = store
            .save(&GitOperationJournal::default())
            .expect_err("Windows Git journal writes must remain unsupported");

        assert!(error.contains("unsupported on this platform"));
        assert!(!app_data_dir.join(JOURNAL_DIRECTORY).exists());
        assert!(!store.path().exists());
        Ok(())
    }
}
