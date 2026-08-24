use std::io::{Read as _, Write as _};

use atomic_write_file::AtomicWriteFile;

use super::*;

impl CodeThreadBindingStore {
    /// Open the binding store rooted at an existing application-data
    /// directory, creating its private real `code` child when absent.
    ///
    /// The app-data directory itself and its `code` child must not be symlinks.
    pub fn for_app_data(app_data_dir: &Path) -> Result<Self, String> {
        if !app_data_dir.is_absolute() {
            return Err("SchoolX Code app-data directory must be absolute".to_string());
        }
        let app_metadata = fs::symlink_metadata(app_data_dir).map_err(|error| {
            format!(
                "failed to inspect SchoolX Code app-data directory {}: {error}",
                app_data_dir.display()
            )
        })?;
        if app_metadata.file_type().is_symlink() {
            return Err("SchoolX Code app-data directory cannot be a symlink".to_string());
        }
        if !app_metadata.is_dir() {
            return Err("SchoolX Code app-data path is not a directory".to_string());
        }

        let app_data_dir = app_data_dir.canonicalize().map_err(|error| {
            format!("failed to resolve SchoolX Code app-data directory: {error}")
        })?;
        let code_dir = app_data_dir.join(CODE_STORE_DIRECTORY);
        ensure_private_real_directory(&code_dir)?;
        let code_dir = code_dir
            .canonicalize()
            .map_err(|error| format!("failed to resolve SchoolX Code data directory: {error}"))?;
        if code_dir.parent() != Some(app_data_dir.as_path()) || !code_dir.starts_with(&app_data_dir)
        {
            return Err("SchoolX Code data directory escaped the app-data root".to_string());
        }

        let store = Self {
            store_path: code_dir.join(CODE_BINDING_STORE_FILE),
            app_data_dir,
            code_dir,
            read_only: false,
        };
        store.validate_store_paths()?;
        Ok(store)
    }

    /// Open an existing binding store without creating directories or changing
    /// permissions. An absent private `code` directory represents an empty
    /// inventory and is returned as `None`.
    ///
    /// Read-only projections use this constructor so a list call cannot turn a
    /// previously untouched app-data directory into durable SchoolX state.
    pub(crate) fn for_app_data_read_only(app_data_dir: &Path) -> Result<Option<Self>, String> {
        if !app_data_dir.is_absolute() {
            return Err("SchoolX Code app-data directory must be absolute".to_string());
        }
        let app_metadata = fs::symlink_metadata(app_data_dir).map_err(|error| {
            format!(
                "failed to inspect SchoolX Code app-data directory {}: {error}",
                app_data_dir.display()
            )
        })?;
        if app_metadata.file_type().is_symlink() {
            return Err("SchoolX Code app-data directory cannot be a symlink".to_string());
        }
        if !app_metadata.is_dir() {
            return Err("SchoolX Code app-data path is not a directory".to_string());
        }

        let app_data_dir = app_data_dir.canonicalize().map_err(|error| {
            format!("failed to resolve SchoolX Code app-data directory: {error}")
        })?;
        let expected_code_dir = app_data_dir.join(CODE_STORE_DIRECTORY);
        let code_metadata = match fs::symlink_metadata(&expected_code_dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(format!(
                    "failed to inspect SchoolX Code data directory {}: {error}",
                    expected_code_dir.display()
                ));
            }
        };
        if code_metadata.file_type().is_symlink() {
            return Err(format!(
                "SchoolX Code data directory {} cannot be a symlink",
                expected_code_dir.display()
            ));
        }
        if !code_metadata.is_dir() {
            return Err(format!(
                "SchoolX Code data path {} is not a directory",
                expected_code_dir.display()
            ));
        }
        let code_dir = expected_code_dir
            .canonicalize()
            .map_err(|error| format!("failed to resolve SchoolX Code data directory: {error}"))?;
        if code_dir != expected_code_dir {
            return Err("SchoolX Code data directory escaped the app-data root".to_string());
        }

        let store = Self {
            store_path: code_dir.join(CODE_BINDING_STORE_FILE),
            app_data_dir,
            code_dir,
            read_only: true,
        };
        store.validate_store_paths()?;
        Ok(Some(store))
    }

    /// Return the canonical path of the binding index for focused persistence tests.
    #[cfg(test)]
    pub fn store_path(&self) -> &Path {
        &self.store_path
    }

    /// Load and validate the complete binding index.
    ///
    /// An absent file is a new empty current-version index. Invalid JSON, a missing
    /// version, unsupported schema versions, and invalid or duplicate records
    /// are errors; the original file is never rewritten during load.
    pub fn load(&self) -> Result<CodeThreadBindingIndex, String> {
        self.validate_store_paths()?;
        let file = match open_binding_index(&self.store_path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(CodeThreadBindingIndex::default());
            }
            Err(error) => {
                return Err(format!(
                    "failed to open SchoolX Code binding index: {error}"
                ));
            }
        };
        let metadata = file
            .metadata()
            .map_err(|error| format!("failed to inspect SchoolX Code binding index: {error}"))?;
        if !metadata.is_file() {
            return Err("SchoolX Code binding index path is not a regular file".to_string());
        }
        if self.read_only {
            validate_read_only_binding_file(&self.app_data_dir, &metadata)?;
        }
        if metadata.len() > MAX_BINDING_STORE_BYTES {
            return Err(format!(
                "SchoolX Code binding index exceeds the {MAX_BINDING_STORE_BYTES}-byte limit"
            ));
        }
        // Validate the named parents and target again after acquiring the file
        // handle. Parsing below uses only this opened handle, so a later path
        // replacement cannot redirect the read to a different file.
        self.validate_store_paths()?;

        let mut bytes = Vec::new();
        file.take(MAX_BINDING_STORE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("failed to read SchoolX Code binding index: {error}"))?;
        if bytes.len() as u64 > MAX_BINDING_STORE_BYTES {
            return Err(format!(
                "SchoolX Code binding index exceeds the {MAX_BINDING_STORE_BYTES}-byte limit"
            ));
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|error| format!("SchoolX Code binding index is invalid JSON: {error}"))?;
        if value.get("version").and_then(serde_json::Value::as_u64)
            == Some(u64::from(CODE_THREAD_BINDING_SCHEMA_VERSION))
        {
            removal::validate_v4_removal_wire(&bytes)?;
        }
        let mut index = lifecycle::decode_binding_index(value)?;
        index.validate()?;
        index.sort();
        Ok(index)
    }

    /// List all bindings in one exact community/project/repository scope.
    pub fn list(&self, scope: &CodeThreadBindingScope) -> Result<Vec<CodeThreadBinding>, String> {
        scope.validate()?;
        Ok(self
            .load()?
            .bindings
            .into_iter()
            .filter(|binding| binding.is_in_scope(scope))
            .collect())
    }

    /// Look up one thread only when its complete isolation scope matches.
    ///
    /// A thread that exists in another scope is reported as absent rather than
    /// leaking or silently re-binding it into the requested scope.
    pub fn lookup(
        &self,
        input: &CodeThreadBindingLookupInput,
    ) -> Result<Option<CodeThreadBinding>, String> {
        input.validate()?;
        Ok(self.load()?.bindings.into_iter().find(|binding| {
            binding.codex_thread_id == input.codex_thread_id && binding.is_in_scope(&input.scope)
        }))
    }

    /// Load the optional native merge target joined to one exact binding.
    /// Callers still need mandatory descriptor and Git revalidation before
    /// treating the ref as graph evidence.
    #[allow(dead_code)]
    pub(crate) fn binding_merge_authority(
        &self,
        input: &CodeThreadBindingLookupInput,
    ) -> Result<Option<(CodeThreadBinding, Option<String>)>, String> {
        input.validate()?;
        let index = self.load()?;
        let Some(binding) = index.bindings.iter().find(|binding| {
            binding.codex_thread_id == input.codex_thread_id && binding.is_in_scope(&input.scope)
        }) else {
            return Ok(None);
        };
        let target_ref = index
            .merge_targets
            .iter()
            .find(|authority| authority.lookup() == *input)
            .map(|authority| authority.target_ref.clone());
        Ok(Some((binding.clone(), target_ref)))
    }

    /// Fail when a prepared managed worktree is already owned by any persisted
    /// Codex thread, regardless of community/project/repository scope.
    ///
    /// Local checkouts intentionally remain shareable by multiple threads.
    /// Callers must hold the application-level binding-store mutex across this
    /// check, `thread/start`, and [`Self::upsert`] to close the precheck/commit
    /// race.
    #[cfg(test)]
    pub fn ensure_execution_available(
        &self,
        input: &CodeExecutionAvailabilityInput,
    ) -> Result<(), String> {
        input.validate()?;
        let index = self.load()?;
        if removal::reserves_execution(&index, input.worktree_id.as_deref(), &input.execution_root)
        {
            return Err(
                "SchoolX Code execution identity is permanently reserved by removal state"
                    .to_string(),
            );
        }
        if input.execution_mode == CodeExecutionMode::Local {
            return Ok(());
        }
        let worktree_id = input.worktree_id.as_deref().ok_or_else(|| {
            "SchoolX Code worktree execution is missing its worktree id".to_string()
        })?;
        let bound = index.bindings.iter().any(|existing| {
            existing.execution_mode == CodeExecutionMode::Worktree
                && (existing.worktree_id.as_deref() == Some(worktree_id)
                    || existing.execution_root == input.execution_root)
        });
        let prepared = index.preparations.iter().any(|existing| {
            existing.execution_mode == CodeExecutionMode::Worktree
                && (existing.worktree_id.as_deref() == Some(worktree_id)
                    || existing.execution_root == input.execution_root)
        });
        if bound || prepared {
            return Err(format!(
                "managed worktree {worktree_id} is already bound to a Codex thread"
            ));
        }
        Ok(())
    }

    /// Atomically add one validated binding to the index.
    ///
    /// Repeating the exact same binding is idempotent. Reusing a Codex thread
    /// id for a different scope/root or assigning one managed worktree to a
    /// second thread fails without changing the existing index.
    #[cfg(test)]
    pub fn upsert(&self, binding: CodeThreadBinding) -> Result<CodeThreadBinding, String> {
        binding.validate()?;
        validate_live_execution_root(&binding.execution_root)?;
        let mut index = self.load()?;

        if removal::reserves_thread_id(&index, &binding.codex_thread_id) {
            return Err(format!(
                "Codex thread {} is permanently reserved by SchoolX Code removal state",
                binding.codex_thread_id
            ));
        }
        if removal::reserves_execution(
            &index,
            binding.worktree_id.as_deref(),
            &binding.execution_root,
        ) {
            return Err(
                "SchoolX Code binding reuses an execution identity reserved by removal state"
                    .to_string(),
            );
        }

        if let Some(existing) = index
            .bindings
            .iter()
            .find(|existing| existing.codex_thread_id == binding.codex_thread_id)
        {
            if existing == &binding {
                return Ok(existing.clone());
            }
            return Err(format!(
                "Codex thread {} is already bound to a different SchoolX Code scope or execution root",
                binding.codex_thread_id
            ));
        }

        if binding.execution_mode == CodeExecutionMode::Worktree {
            let worktree_id = binding.worktree_id.as_deref().ok_or_else(|| {
                "SchoolX Code worktree binding is missing its worktree id".to_string()
            })?;
            let bound = index.bindings.iter().any(|existing| {
                existing.execution_mode == CodeExecutionMode::Worktree
                    && (existing.worktree_id.as_deref() == Some(worktree_id)
                        || existing.execution_root == binding.execution_root)
            });
            let prepared = index.preparations.iter().any(|existing| {
                existing.execution_mode == CodeExecutionMode::Worktree
                    && (existing.worktree_id.as_deref() == Some(worktree_id)
                        || existing.execution_root == binding.execution_root)
            });
            if bound || prepared {
                return Err(format!(
                    "managed worktree {worktree_id} is already bound to another Codex thread"
                ));
            }
        }

        index.bindings.push(binding.clone());
        lifecycle::insert_active_lifecycle(&mut index.lifecycles, &binding)?;
        index.sort();
        index.validate()?;
        self.save(&index)?;
        Ok(binding)
    }

    pub(super) fn save(&self, index: &CodeThreadBindingIndex) -> Result<(), String> {
        if self.read_only {
            return Err("read-only SchoolX Code binding store cannot be changed".to_string());
        }
        self.validate_store_paths()?;
        let mut index = index.clone();
        index.sort();
        index.validate()?;
        let mut payload = serde_json::to_vec_pretty(&index)
            .map_err(|error| format!("failed to encode SchoolX Code binding index: {error}"))?;
        payload.push(b'\n');
        if payload.len() as u64 > MAX_BINDING_STORE_BYTES {
            return Err(format!(
                "SchoolX Code binding index exceeds the {MAX_BINDING_STORE_BYTES}-byte limit"
            ));
        }

        // Do not canonicalize the target. The managed-agent store helper does
        // so intentionally to preserve shared-data symlinks, but a Code index
        // must never follow a replacement link outside app data.
        let mut file = AtomicWriteFile::open(&self.store_path)
            .map_err(|error| format!("failed to open SchoolX Code binding index: {error}"))?;
        // Re-check after opening the sibling temporary file so a parent/target
        // replacement between the first validation and open is detected before
        // any binding bytes are written.
        self.validate_store_paths()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| format!("failed to secure SchoolX Code binding index: {error}"))?;
        }
        file.write_all(&payload)
            .map_err(|error| format!("failed to write SchoolX Code binding index: {error}"))?;
        #[cfg(unix)]
        let directory = {
            use std::os::fd::AsFd as _;
            file.directory()
                .ok_or_else(|| {
                    "SchoolX Code binding index has no pinned parent directory".to_string()
                })?
                .as_fd()
                .try_clone_to_owned()
                .map_err(|error| format!("failed to pin SchoolX Code binding directory: {error}"))?
        };
        file.commit()
            .map_err(|error| format!("failed to commit SchoolX Code binding index: {error}"))?;
        #[cfg(unix)]
        rustix::fs::fsync(&directory)
            .map_err(|error| format!("failed to sync SchoolX Code binding directory: {error}"))?;
        Ok(())
    }

    fn validate_store_paths(&self) -> Result<(), String> {
        validate_real_directory(&self.app_data_dir, "app-data")?;
        validate_real_directory(&self.code_dir, "data")?;
        let current_app_data = self.app_data_dir.canonicalize().map_err(|error| {
            format!("failed to resolve SchoolX Code app-data directory: {error}")
        })?;
        if current_app_data != self.app_data_dir {
            return Err("SchoolX Code app-data directory changed after initialization".to_string());
        }
        let current_code = self
            .code_dir
            .canonicalize()
            .map_err(|error| format!("failed to resolve SchoolX Code data directory: {error}"))?;
        if current_code != self.code_dir
            || current_code.parent() != Some(current_app_data.as_path())
            || !current_code.starts_with(&current_app_data)
        {
            return Err("SchoolX Code data directory escaped the app-data root".to_string());
        }
        if self.store_path.parent() != Some(current_code.as_path()) {
            return Err("SchoolX Code binding index escaped its data directory".to_string());
        }
        if self.read_only {
            validate_read_only_store_permissions(
                &self.app_data_dir,
                &self.code_dir,
                &self.store_path,
            )?;
        }

        match fs::symlink_metadata(&self.store_path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                Err("SchoolX Code binding index cannot be a symlink".to_string())
            }
            Ok(metadata) if !metadata.is_file() => {
                Err("SchoolX Code binding index path is not a file".to_string())
            }
            Ok(_) => {
                let resolved = self.store_path.canonicalize().map_err(|error| {
                    format!("failed to resolve SchoolX Code binding index: {error}")
                })?;
                if resolved.parent() != Some(current_code.as_path())
                    || !resolved.starts_with(&current_code)
                {
                    return Err("SchoolX Code binding index escaped its data directory".to_string());
                }
                Ok(())
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "failed to inspect SchoolX Code binding index: {error}"
            )),
        }
    }
}

#[cfg(unix)]
fn open_binding_index(path: &Path) -> std::io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    fs::OpenOptions::new()
        .read(true)
        // O_NOFOLLOW closes the final-component lstat/open race. O_NONBLOCK
        // prevents a replacement FIFO or device from blocking before fstat
        // confirms that the opened handle is a regular file.
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(not(unix))]
fn open_binding_index(path: &Path) -> std::io::Result<fs::File> {
    fs::OpenOptions::new().read(true).open(path)
}

fn ensure_private_real_directory(path: &Path) -> Result<(), String> {
    let needs_create = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(format!(
                "SchoolX Code data directory {} cannot be a symlink",
                path.display()
            ));
        }
        Ok(metadata) if !metadata.is_dir() => {
            return Err(format!(
                "SchoolX Code data path {} is not a directory",
                path.display()
            ));
        }
        Ok(_) => false,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => {
            return Err(format!(
                "failed to inspect SchoolX Code data directory {}: {error}",
                path.display()
            ));
        }
    };

    if needs_create {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            match builder.create(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(format!(
                        "failed to create SchoolX Code data directory {}: {error}",
                        path.display()
                    ));
                }
            }
        }
        #[cfg(not(unix))]
        match fs::create_dir(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(format!(
                    "failed to create SchoolX Code data directory {}: {error}",
                    path.display()
                ));
            }
        }
    }

    validate_real_directory(path, "data")?;
    restrict_directory_to_owner(path)
}

#[cfg(unix)]
fn restrict_directory_to_owner(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

    let directory = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| {
            format!(
                "failed to securely open SchoolX Code data directory {}: {error}",
                path.display()
            )
        })?;
    let metadata = directory.metadata().map_err(|error| {
        format!(
            "failed to inspect open SchoolX Code data directory {}: {error}",
            path.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err("SchoolX Code data path is not a directory".to_string());
    }
    directory
        .set_permissions(fs::Permissions::from_mode(0o700))
        .map_err(|error| {
            format!(
                "failed to secure SchoolX Code data directory {}: {error}",
                path.display()
            )
        })
}

#[cfg(not(unix))]
fn restrict_directory_to_owner(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn validate_real_directory(path: &Path, label: &str) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        format!(
            "failed to inspect SchoolX Code {label} directory {}: {error}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "SchoolX Code {label} directory cannot be a symlink"
        ));
    }
    if !metadata.is_dir() {
        return Err(format!("SchoolX Code {label} path is not a directory"));
    }
    Ok(())
}

#[cfg(unix)]
fn validate_read_only_store_permissions(
    app_data_dir: &Path,
    code_dir: &Path,
    store_path: &Path,
) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt as _;

    let app_metadata = fs::symlink_metadata(app_data_dir)
        .map_err(|error| format!("failed to inspect SchoolX Code app-data ownership: {error}"))?;
    let code_metadata = fs::symlink_metadata(code_dir)
        .map_err(|error| format!("failed to inspect SchoolX Code data permissions: {error}"))?;
    if code_metadata.uid() != app_metadata.uid() {
        return Err("SchoolX Code data directory has an unexpected owner".to_string());
    }
    if code_metadata.mode() & 0o7777 != 0o700 {
        return Err(
            "SchoolX Code data directory is not private; read-only inventory refused it"
                .to_string(),
        );
    }

    match fs::symlink_metadata(store_path) {
        Ok(metadata) => validate_read_only_binding_file(app_data_dir, &metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to inspect SchoolX Code binding index permissions: {error}"
        )),
    }
}

#[cfg(not(unix))]
fn validate_read_only_store_permissions(
    _app_data_dir: &Path,
    _code_dir: &Path,
    _store_path: &Path,
) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn validate_read_only_binding_file(
    app_data_dir: &Path,
    metadata: &fs::Metadata,
) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt as _;

    let app_metadata = fs::symlink_metadata(app_data_dir)
        .map_err(|error| format!("failed to inspect SchoolX Code app-data ownership: {error}"))?;
    if metadata.uid() != app_metadata.uid() {
        return Err("SchoolX Code binding index has an unexpected owner".to_string());
    }
    if metadata.mode() & 0o7777 != 0o600 {
        return Err(
            "SchoolX Code binding index is not private; read-only inventory refused it".to_string(),
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_read_only_binding_file(
    _app_data_dir: &Path,
    _metadata: &fs::Metadata,
) -> Result<(), String> {
    Ok(())
}
