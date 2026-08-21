//! Durable SchoolX Code thread-to-execution-root bindings.
//!
//! Codex owns the thread transcript. SchoolX persists only the narrow index
//! needed to recover the native execution boundary after an app restart. The
//! stored execution root is authoritative once a thread is bound; callers may
//! not substitute a new path while resuming the thread or starting a turn.

use std::collections::HashSet;
use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Component, Path, PathBuf};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};

use super::worktrees::CodeWorktreeDescriptor;

mod lifecycle;
pub(crate) mod removal;

pub use lifecycle::CodeThreadLifecycleStatus;
pub(crate) use lifecycle::{CodeThreadBindingLifecycle, CodeThreadLifecycleClaim};

/// Schema version written to every SchoolX Code binding index.
pub const CODE_THREAD_BINDING_SCHEMA_VERSION: u32 = 4;

const CODE_STORE_DIRECTORY: &str = "code";
const CODE_BINDING_STORE_FILE: &str = "thread-bindings.json";
const MAX_BINDING_STORE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_BINDINGS: usize = 4_096;
const MAX_PREPARATIONS: usize = 4_096;
const MAX_MERGE_TARGETS: usize = 4_096;
const MAX_RECOVERY_BASELINE_THREADS: usize = 4_096;
const MAX_IDENTIFIER_BYTES: usize = 512;
const MAX_EXECUTION_ROOT_BYTES: usize = 32 * 1024;

/// Whether Codex executes in a SchoolX-managed worktree or an explicit local
/// checkout selected by the user.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CodeExecutionMode {
    /// A detached worktree below the current SchoolX nest's `WORKTREES` root.
    Worktree,
    /// The user's existing checkout, which SchoolX must never switch or reset.
    Local,
}

/// The community, project, and native repository coordinate that isolates a
/// group of SchoolX Code bindings.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeThreadBindingScope {
    /// Product-local community identifier.
    pub community_id: String,
    /// Nostr project `d` tag.
    pub project_dtag: String,
    /// Lowercase SHA-256 identity derived from the canonical Git common-dir.
    pub repository_identity: String,
}

impl CodeThreadBindingScope {
    pub(crate) fn validate(&self) -> Result<(), String> {
        validate_identifier("community", &self.community_id)?;
        validate_identifier("project dtag", &self.project_dtag)?;
        validate_sha256("repository identity", &self.repository_identity)
    }
}

/// Exact lookup coordinate for one persisted Codex thread binding.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeThreadBindingLookupInput {
    /// Community/project/repository scope that must own the thread.
    pub scope: CodeThreadBindingScope,
    /// Opaque Codex app-server thread identifier.
    pub codex_thread_id: String,
}

impl CodeThreadBindingLookupInput {
    fn validate(&self) -> Result<(), String> {
        self.scope.validate()?;
        validate_identifier("Codex thread", &self.codex_thread_id)
    }
}

/// Native execution descriptor used by focused store invariant tests.
#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeExecutionAvailabilityInput {
    /// Execution mode selected during native preparation.
    pub execution_mode: CodeExecutionMode,
    /// Canonical native execution root.
    pub execution_root: String,
    /// Managed worktree UUID, absent for an explicit local checkout.
    pub worktree_id: Option<String>,
}

#[cfg(test)]
impl CodeExecutionAvailabilityInput {
    fn validate(&self) -> Result<(), String> {
        validate_execution_root(&self.execution_root)?;
        validate_live_execution_root(&self.execution_root)?;
        match (self.execution_mode, self.worktree_id.as_deref()) {
            (CodeExecutionMode::Worktree, Some(worktree_id)) => validate_worktree_id(worktree_id),
            (CodeExecutionMode::Worktree, None) => {
                Err("SchoolX Code worktree execution requires a worktree id".to_string())
            }
            (CodeExecutionMode::Local, None) => Ok(()),
            (CodeExecutionMode::Local, Some(_)) => {
                Err("SchoolX Code local execution cannot carry a worktree id".to_string())
            }
        }
    }
}

/// Durable native binding between a Codex thread and one execution root.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeThreadBinding {
    /// Product-local community identifier.
    pub community_id: String,
    /// Nostr project `d` tag.
    pub project_dtag: String,
    /// Lowercase SHA-256 identity derived from the canonical Git common-dir.
    pub repository_identity: String,
    /// Opaque Codex app-server thread identifier.
    pub codex_thread_id: String,
    /// Native execution mode fixed for the life of the binding.
    pub execution_mode: CodeExecutionMode,
    /// Canonical absolute directory passed to Codex as its execution root.
    pub execution_root: String,
    /// Resolved Git commit id used as the thread's base ref.
    pub base_ref: String,
    /// Managed worktree UUID. Must be absent in local-checkout mode.
    pub worktree_id: Option<String>,
}

/// Durable state of a native-issued execution preparation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodeThreadPreparationState {
    /// The execution root is reserved and has not been submitted to Codex.
    Prepared,
    /// `thread/start` was submitted and must not be repeated without recovery.
    Starting,
}

/// Native operation that owns one durable execution preparation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodeThreadPreparationOperation {
    /// A new root thread will be created with `thread/start`.
    Start,
    /// A bound source thread will be copied with `thread/fork`.
    Fork,
}

/// Native-issued execution descriptor retained until a Codex thread binding
/// is committed atomically.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeThreadPreparation {
    /// Opaque canonical UUID returned by `code_worktree_prepare`.
    pub preparation_id: String,
    /// Product-local community identifier.
    pub community_id: String,
    /// Nostr project `d` tag.
    pub project_dtag: String,
    /// Lowercase SHA-256 identity derived from the canonical Git common-dir.
    pub repository_identity: String,
    /// Native execution mode fixed by preparation.
    pub execution_mode: CodeExecutionMode,
    /// Canonical absolute execution root.
    pub execution_root: String,
    /// Resolved immutable Git commit used as the base ref.
    pub base_ref: String,
    /// Managed worktree UUID, absent for an explicit local checkout.
    pub worktree_id: Option<String>,
    /// Exact operation allowed to consume this reservation.
    pub operation: CodeThreadPreparationOperation,
    /// Bound source thread for `fork`; absent for a root `start`.
    pub source_thread_id: Option<String>,
    /// Whether starting the Codex thread is still safe or requires recovery.
    pub state: CodeThreadPreparationState,
    /// Exact-root Codex thread IDs observed before `thread/start` was sent.
    ///
    /// `None` identifies a legacy `starting` record created before native
    /// discovery was available; `Some([])` is a valid empty baseline. This
    /// native-only recovery detail is scrubbed from preparation-list results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) recovery_thread_baseline: Option<Vec<String>>,
    /// Optional native-owned direct local branch used only for a future
    /// managed-worktree ancestry proof. This value is never accepted from or
    /// returned to the webview.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) merge_target_ref: Option<String>,
}

impl CodeThreadPreparation {
    /// Return the exact isolation scope that owns this preparation.
    pub fn scope(&self) -> CodeThreadBindingScope {
        CodeThreadBindingScope {
            community_id: self.community_id.clone(),
            project_dtag: self.project_dtag.clone(),
            repository_identity: self.repository_identity.clone(),
        }
    }

    /// Reconstruct the native descriptor for mandatory filesystem and Git
    /// revalidation before use.
    pub fn descriptor(&self) -> CodeWorktreeDescriptor {
        CodeWorktreeDescriptor {
            execution_mode: self.execution_mode,
            repository_identity: self.repository_identity.clone(),
            execution_root: self.execution_root.clone(),
            base_ref: self.base_ref.clone(),
            worktree_id: self.worktree_id.clone(),
        }
    }

    fn is_in_scope(&self, scope: &CodeThreadBindingScope) -> bool {
        self.community_id == scope.community_id
            && self.project_dtag == scope.project_dtag
            && self.repository_identity == scope.repository_identity
    }

    fn validate(&self) -> Result<(), String> {
        validate_preparation_id(&self.preparation_id)?;
        self.scope().validate()?;
        validate_execution_root(&self.execution_root)?;
        validate_commit_id(&self.base_ref)?;
        match (self.operation, self.source_thread_id.as_deref()) {
            (CodeThreadPreparationOperation::Start, None) => {}
            (CodeThreadPreparationOperation::Start, Some(_)) => {
                return Err(
                    "SchoolX Code start preparation cannot carry a source thread".to_string(),
                );
            }
            (CodeThreadPreparationOperation::Fork, Some(source_thread_id)) => {
                validate_identifier("fork source thread", source_thread_id)?;
                if self.execution_mode != CodeExecutionMode::Worktree {
                    return Err(
                        "SchoolX Code fork preparation must reserve a managed worktree".to_string(),
                    );
                }
            }
            (CodeThreadPreparationOperation::Fork, None) => {
                return Err(
                    "SchoolX Code fork preparation is missing its source thread".to_string()
                );
            }
        }
        match (self.state, self.recovery_thread_baseline.as_ref()) {
            (CodeThreadPreparationState::Prepared, Some(_)) => {
                return Err(
                    "SchoolX Code prepared execution cannot carry a recovery baseline".to_string(),
                );
            }
            (CodeThreadPreparationState::Starting, Some(thread_ids)) => {
                validate_recovery_baseline(thread_ids)?;
            }
            _ => {}
        }
        if self.operation == CodeThreadPreparationOperation::Fork
            && self.state == CodeThreadPreparationState::Starting
            && self.recovery_thread_baseline.is_none()
        {
            return Err(
                "SchoolX Code starting fork preparation requires a recovery baseline".to_string(),
            );
        }
        match (self.execution_mode, self.worktree_id.as_deref()) {
            (CodeExecutionMode::Worktree, Some(worktree_id)) => {
                validate_worktree_id(worktree_id)?;
                if let Some(target_ref) = self.merge_target_ref.as_deref() {
                    validate_direct_local_branch_ref(target_ref)?;
                }
                Ok(())
            }
            (CodeExecutionMode::Worktree, None) => {
                Err("SchoolX Code worktree preparation requires a worktree id".to_string())
            }
            (CodeExecutionMode::Local, None) if self.merge_target_ref.is_none() => Ok(()),
            (CodeExecutionMode::Local, None) => Err(
                "SchoolX Code local preparation cannot carry merge-target authority".to_string(),
            ),
            (CodeExecutionMode::Local, Some(_)) => {
                Err("SchoolX Code local preparation cannot carry a worktree id".to_string())
            }
        }
    }
}

/// Native-only direct-local-branch authority joined to one committed managed
/// binding. The public eight-field binding deliberately remains unchanged.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CodeWorktreeMergeTarget {
    community_id: String,
    project_dtag: String,
    repository_identity: String,
    codex_thread_id: String,
    worktree_id: String,
    target_ref: String,
}

impl CodeWorktreeMergeTarget {
    fn scope(&self) -> CodeThreadBindingScope {
        CodeThreadBindingScope {
            community_id: self.community_id.clone(),
            project_dtag: self.project_dtag.clone(),
            repository_identity: self.repository_identity.clone(),
        }
    }

    fn lookup(&self) -> CodeThreadBindingLookupInput {
        CodeThreadBindingLookupInput {
            scope: self.scope(),
            codex_thread_id: self.codex_thread_id.clone(),
        }
    }

    fn for_binding(binding: &CodeThreadBinding, target_ref: String) -> Result<Self, String> {
        let worktree_id = binding.worktree_id.clone().ok_or_else(|| {
            "SchoolX Code merge-target authority requires a managed worktree".to_string()
        })?;
        let authority = Self {
            community_id: binding.community_id.clone(),
            project_dtag: binding.project_dtag.clone(),
            repository_identity: binding.repository_identity.clone(),
            codex_thread_id: binding.codex_thread_id.clone(),
            worktree_id,
            target_ref,
        };
        authority.validate()?;
        Ok(authority)
    }

    fn validate(&self) -> Result<(), String> {
        self.lookup().validate()?;
        validate_worktree_id(&self.worktree_id)?;
        validate_direct_local_branch_ref(&self.target_ref)
    }
}

impl CodeThreadBinding {
    /// Return the isolation scope represented by this binding.
    pub fn scope(&self) -> CodeThreadBindingScope {
        CodeThreadBindingScope {
            community_id: self.community_id.clone(),
            project_dtag: self.project_dtag.clone(),
            repository_identity: self.repository_identity.clone(),
        }
    }

    fn is_in_scope(&self, scope: &CodeThreadBindingScope) -> bool {
        self.community_id == scope.community_id
            && self.project_dtag == scope.project_dtag
            && self.repository_identity == scope.repository_identity
    }

    fn validate(&self) -> Result<(), String> {
        self.scope().validate()?;
        validate_identifier("Codex thread", &self.codex_thread_id)?;
        validate_execution_root(&self.execution_root)?;
        validate_commit_id(&self.base_ref)?;

        match (self.execution_mode, self.worktree_id.as_deref()) {
            (CodeExecutionMode::Worktree, Some(worktree_id)) => validate_worktree_id(worktree_id),
            (CodeExecutionMode::Worktree, None) => {
                Err("SchoolX Code worktree bindings require a worktree id".to_string())
            }
            (CodeExecutionMode::Local, None) => Ok(()),
            (CodeExecutionMode::Local, Some(_)) => {
                Err("SchoolX Code local bindings cannot carry a worktree id".to_string())
            }
        }
    }
}

/// Versioned on-disk SchoolX Code binding index.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeThreadBindingIndex {
    /// Exact schema version for this file.
    pub version: u32,
    /// All persisted bindings, stored in deterministic coordinate order.
    pub bindings: Vec<CodeThreadBinding>,
    /// Native-only lifecycle projections atomically joined one-to-one with bindings.
    pub(crate) lifecycles: Vec<lifecycle::CodeThreadLifecycleRecord>,
    /// Native-issued execution reservations not yet committed as bindings.
    #[serde(default)]
    pub preparations: Vec<CodeThreadPreparation>,
    /// Optional native merge target for committed managed bindings.
    pub(crate) merge_targets: Vec<CodeWorktreeMergeTarget>,
    /// Native-only removal claims and permanent transcript tombstones.
    pub(crate) removals: Vec<removal::CodeWorktreeRemovalRecord>,
}

impl Default for CodeThreadBindingIndex {
    fn default() -> Self {
        Self {
            version: CODE_THREAD_BINDING_SCHEMA_VERSION,
            bindings: Vec::new(),
            lifecycles: Vec::new(),
            preparations: Vec::new(),
            merge_targets: Vec::new(),
            removals: Vec::new(),
        }
    }
}

impl CodeThreadBindingIndex {
    fn validate(&self) -> Result<(), String> {
        if self.version != CODE_THREAD_BINDING_SCHEMA_VERSION {
            return Err(format!(
                "unsupported SchoolX Code binding schema version {}",
                self.version
            ));
        }
        if self.bindings.len() > MAX_BINDINGS {
            return Err(format!(
                "SchoolX Code binding index exceeds the {MAX_BINDINGS}-binding limit"
            ));
        }
        if self.preparations.len() > MAX_PREPARATIONS {
            return Err(format!(
                "SchoolX Code binding index exceeds the {MAX_PREPARATIONS}-preparation limit"
            ));
        }
        if self.merge_targets.len() > MAX_MERGE_TARGETS {
            return Err(format!(
                "SchoolX Code binding index exceeds the {MAX_MERGE_TARGETS}-merge-target limit"
            ));
        }

        let mut thread_ids = HashSet::with_capacity(self.bindings.len());
        let mut managed_worktree_ids = HashSet::new();
        let mut managed_execution_roots = HashSet::new();
        for binding in &self.bindings {
            binding.validate()?;
            if !thread_ids.insert(binding.codex_thread_id.as_str()) {
                return Err(format!(
                    "SchoolX Code binding index contains duplicate Codex thread id {}",
                    binding.codex_thread_id
                ));
            }
            if binding.execution_mode == CodeExecutionMode::Worktree {
                let worktree_id = binding.worktree_id.as_deref().ok_or_else(|| {
                    "SchoolX Code worktree binding is missing its worktree id".to_string()
                })?;
                if !managed_worktree_ids.insert(worktree_id) {
                    return Err(format!(
                        "SchoolX Code binding index contains duplicate managed worktree id {worktree_id}"
                    ));
                }
                if !managed_execution_roots.insert(binding.execution_root.as_str()) {
                    return Err(format!(
                        "SchoolX Code binding index contains duplicate managed execution root {}",
                        binding.execution_root
                    ));
                }
            }
        }
        lifecycle::validate_lifecycle_join(&self.bindings, &self.lifecycles)?;
        let mut preparation_ids = HashSet::with_capacity(self.preparations.len());
        let mut unfinished_fork_sources = HashSet::new();
        for preparation in &self.preparations {
            preparation.validate()?;
            if !preparation_ids.insert(preparation.preparation_id.as_str()) {
                return Err(format!(
                    "SchoolX Code binding index contains duplicate preparation id {}",
                    preparation.preparation_id
                ));
            }
            if preparation.execution_mode == CodeExecutionMode::Worktree {
                let worktree_id = preparation.worktree_id.as_deref().ok_or_else(|| {
                    "SchoolX Code worktree preparation is missing its worktree id".to_string()
                })?;
                if !managed_worktree_ids.insert(worktree_id) {
                    return Err(format!(
                        "SchoolX Code binding index contains duplicate managed worktree id {worktree_id}"
                    ));
                }
                if !managed_execution_roots.insert(preparation.execution_root.as_str()) {
                    return Err(format!(
                        "SchoolX Code binding index contains duplicate managed execution root {}",
                        preparation.execution_root
                    ));
                }
            }
            if preparation.operation == CodeThreadPreparationOperation::Fork {
                let source_thread_id =
                    preparation.source_thread_id.as_deref().ok_or_else(|| {
                        "SchoolX Code fork preparation is missing its source thread".to_string()
                    })?;
                let source_key = (
                    preparation.community_id.as_str(),
                    preparation.project_dtag.as_str(),
                    preparation.repository_identity.as_str(),
                    source_thread_id,
                );
                if !unfinished_fork_sources.insert(source_key) {
                    return Err(
                        "SchoolX Code binding index contains duplicate unfinished forks for one source"
                            .to_string(),
                    );
                }
                let source = self
                    .bindings
                    .iter()
                    .find(|binding| {
                        binding.community_id == preparation.community_id
                            && binding.project_dtag == preparation.project_dtag
                            && binding.repository_identity == preparation.repository_identity
                            && binding.codex_thread_id == source_thread_id
                    })
                    .ok_or_else(|| {
                        "SchoolX Code fork preparation references a missing source binding"
                            .to_string()
                    })?;
                if source.execution_mode != CodeExecutionMode::Worktree
                    || source.worktree_id.is_none()
                {
                    return Err(
                        "SchoolX Code fork preparation source is not a managed worktree"
                            .to_string(),
                    );
                }
                if source.execution_root == preparation.execution_root
                    || source.worktree_id == preparation.worktree_id
                {
                    return Err(
                        "SchoolX Code fork source and destination share a managed worktree"
                            .to_string(),
                    );
                }
                let source_target = self
                    .merge_targets
                    .iter()
                    .find(|authority| {
                        authority.codex_thread_id == source_thread_id
                            && authority.community_id == preparation.community_id
                            && authority.project_dtag == preparation.project_dtag
                            && authority.repository_identity == preparation.repository_identity
                    })
                    .map(|authority| authority.target_ref.as_str());
                if preparation.merge_target_ref.as_deref() != source_target {
                    return Err(
                        "SchoolX Code fork preparation did not copy its source merge-target authority"
                            .to_string(),
                    );
                }
            }
        }
        let mut merge_target_owners = HashSet::with_capacity(self.merge_targets.len());
        for authority in &self.merge_targets {
            authority.validate()?;
            let lookup = authority.lookup();
            if !merge_target_owners.insert(lookup.clone()) {
                return Err(format!(
                    "SchoolX Code binding index contains duplicate merge-target authority for {}",
                    lookup.codex_thread_id
                ));
            }
            let binding = self
                .bindings
                .iter()
                .find(|binding| {
                    binding.codex_thread_id == lookup.codex_thread_id
                        && binding.is_in_scope(&lookup.scope)
                })
                .ok_or_else(|| {
                    format!(
                        "SchoolX Code binding index contains orphan merge-target authority for {}",
                        lookup.codex_thread_id
                    )
                })?;
            if binding.execution_mode != CodeExecutionMode::Worktree
                || binding.worktree_id.as_deref() != Some(authority.worktree_id.as_str())
            {
                return Err(format!(
                    "SchoolX Code merge-target authority does not match managed binding {}",
                    lookup.codex_thread_id
                ));
            }
        }
        removal::validate_removal_join(self)?;
        Ok(())
    }

    fn sort(&mut self) {
        self.bindings.sort_by(|left, right| {
            left.community_id
                .cmp(&right.community_id)
                .then_with(|| left.project_dtag.cmp(&right.project_dtag))
                .then_with(|| left.repository_identity.cmp(&right.repository_identity))
                .then_with(|| left.codex_thread_id.cmp(&right.codex_thread_id))
        });
        lifecycle::sort_lifecycle_records(&mut self.lifecycles);
        self.preparations.sort_by(|left, right| {
            left.community_id
                .cmp(&right.community_id)
                .then_with(|| left.project_dtag.cmp(&right.project_dtag))
                .then_with(|| left.repository_identity.cmp(&right.repository_identity))
                .then_with(|| left.preparation_id.cmp(&right.preparation_id))
        });
        self.merge_targets.sort_by(|left, right| {
            left.community_id
                .cmp(&right.community_id)
                .then_with(|| left.project_dtag.cmp(&right.project_dtag))
                .then_with(|| left.repository_identity.cmp(&right.repository_identity))
                .then_with(|| left.codex_thread_id.cmp(&right.codex_thread_id))
        });
        removal::sort_removal_records(&mut self.removals);
    }

    /// Return every Codex thread identity that cannot be adopted by recovery.
    /// Removed tombstones remain reserved even though they are not live bindings.
    pub(crate) fn reserved_thread_ids(&self) -> HashSet<String> {
        let mut reserved = self
            .bindings
            .iter()
            .map(|binding| binding.codex_thread_id.clone())
            .collect::<HashSet<_>>();
        reserved.extend(removal::reserved_thread_ids(self));
        reserved
    }
}

/// Filesystem-backed owner of the SchoolX Code binding index.
///
/// A store value is cheap to construct. Callers performing concurrent
/// read-modify-write operations must hold the application-level binding-store
/// mutex across the complete native mutation sequence.
#[derive(Clone, Debug)]
pub struct CodeThreadBindingStore {
    app_data_dir: PathBuf,
    code_dir: PathBuf,
    store_path: PathBuf,
    read_only: bool,
}

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

    /// Fail before filesystem preparation when the durable journal has no room
    /// for another native-issued reservation.
    pub(crate) fn ensure_preparation_capacity(&self) -> Result<(), String> {
        if self.load()?.preparations.len() >= MAX_PREPARATIONS {
            return Err(format!(
                "SchoolX Code binding index reached the {MAX_PREPARATIONS}-preparation limit"
            ));
        }
        Ok(())
    }

    /// Fail before Git mutation when one source already owns unfinished fork work.
    pub(crate) fn ensure_fork_source_available(
        &self,
        scope: &CodeThreadBindingScope,
        source_thread_id: &str,
    ) -> Result<(), String> {
        scope.validate()?;
        validate_identifier("fork source thread", source_thread_id)?;
        let index = self.load()?;
        if removal::reserves_thread_id(&index, source_thread_id) {
            return Err(
                "SchoolX Code source thread is reserved by removal state and cannot be forked"
                    .to_string(),
            );
        }
        if index.preparations.iter().any(|preparation| {
            preparation.is_in_scope(scope)
                && preparation.operation == CodeThreadPreparationOperation::Fork
                && preparation.source_thread_id.as_deref() == Some(source_thread_id)
        }) {
            return Err(
                "SchoolX Code source thread already has an unfinished fork preparation".to_string(),
            );
        }
        Ok(())
    }

    /// Resolve the globally unique owner of one Codex thread for native event
    /// scope enrichment. User-facing commands must continue to use
    /// [`Self::lookup`] with an explicit complete scope.
    pub(crate) fn lookup_thread_id(
        &self,
        codex_thread_id: &str,
    ) -> Result<Option<CodeThreadBinding>, String> {
        validate_identifier("Codex thread", codex_thread_id)?;
        Ok(self
            .load()?
            .bindings
            .into_iter()
            .find(|binding| binding.codex_thread_id == codex_thread_id))
    }

    /// Fail closed when a recovery candidate is already owned by any scope.
    pub(crate) fn ensure_thread_unbound(&self, codex_thread_id: &str) -> Result<(), String> {
        validate_identifier("Codex thread", codex_thread_id)?;
        let index = self.load()?;
        if index
            .bindings
            .iter()
            .any(|binding| binding.codex_thread_id == codex_thread_id)
            || removal::reserves_thread_id(&index, codex_thread_id)
        {
            return Err(format!(
                "Codex thread {codex_thread_id} is already bound to SchoolX Code"
            ));
        }
        Ok(())
    }

    /// Persist a native-issued execution preparation before it may cross the
    /// Codex `thread/start` boundary.
    #[cfg(test)]
    pub(crate) fn create_preparation(
        &self,
        preparation_id: String,
        scope: CodeThreadBindingScope,
        descriptor: &CodeWorktreeDescriptor,
    ) -> Result<CodeThreadPreparation, String> {
        self.create_preparation_with_merge_target(preparation_id, scope, descriptor, None)
    }

    /// Persist a preparation together with native-captured merge authority.
    /// The target is never reconstructed from a caller-supplied descriptor.
    pub(crate) fn create_preparation_with_merge_target(
        &self,
        preparation_id: String,
        scope: CodeThreadBindingScope,
        descriptor: &CodeWorktreeDescriptor,
        merge_target_ref: Option<String>,
    ) -> Result<CodeThreadPreparation, String> {
        scope.validate()?;
        if scope.repository_identity != descriptor.repository_identity {
            return Err(
                "SchoolX Code preparation scope does not match the native repository".to_string(),
            );
        }
        validate_execution_root(&descriptor.execution_root)?;
        validate_live_execution_root(&descriptor.execution_root)?;
        let preparation = CodeThreadPreparation {
            preparation_id,
            community_id: scope.community_id,
            project_dtag: scope.project_dtag,
            repository_identity: scope.repository_identity,
            execution_mode: descriptor.execution_mode,
            execution_root: descriptor.execution_root.clone(),
            base_ref: descriptor.base_ref.clone(),
            worktree_id: descriptor.worktree_id.clone(),
            operation: CodeThreadPreparationOperation::Start,
            source_thread_id: None,
            state: CodeThreadPreparationState::Prepared,
            recovery_thread_baseline: None,
            merge_target_ref,
        };
        preparation.validate()?;

        let mut index = self.load()?;
        if index.preparations.len() >= MAX_PREPARATIONS {
            return Err(format!(
                "SchoolX Code binding index reached the {MAX_PREPARATIONS}-preparation limit"
            ));
        }
        if index
            .preparations
            .iter()
            .any(|existing| existing.preparation_id == preparation.preparation_id)
        {
            return Err("SchoolX Code preparation id is already reserved".to_string());
        }
        ensure_managed_execution_unreserved(&index, &preparation)?;
        index.preparations.push(preparation.clone());
        index.sort();
        index.validate()?;
        self.save(&index)?;
        Ok(preparation)
    }

    /// Persist a fresh managed destination before it may cross `thread/fork`.
    pub(crate) fn create_fork_preparation(
        &self,
        preparation_id: String,
        scope: CodeThreadBindingScope,
        source_thread_id: String,
        descriptor: &CodeWorktreeDescriptor,
    ) -> Result<CodeThreadPreparation, String> {
        scope.validate()?;
        validate_identifier("fork source thread", &source_thread_id)?;
        if descriptor.execution_mode != CodeExecutionMode::Worktree {
            return Err("SchoolX Code fork destination must be a managed worktree".to_string());
        }
        if scope.repository_identity != descriptor.repository_identity {
            return Err(
                "SchoolX Code fork preparation scope does not match the native repository"
                    .to_string(),
            );
        }
        validate_execution_root(&descriptor.execution_root)?;
        validate_live_execution_root(&descriptor.execution_root)?;
        let mut index = self.load()?;
        if removal::reserves_thread_id(&index, &source_thread_id) {
            return Err("SchoolX Code fork source is reserved by removal state".to_string());
        }
        let merge_target_ref = index
            .merge_targets
            .iter()
            .find(|authority| {
                authority.community_id == scope.community_id
                    && authority.project_dtag == scope.project_dtag
                    && authority.repository_identity == scope.repository_identity
                    && authority.codex_thread_id == source_thread_id
            })
            .map(|authority| authority.target_ref.clone());
        let preparation = CodeThreadPreparation {
            preparation_id,
            community_id: scope.community_id,
            project_dtag: scope.project_dtag,
            repository_identity: scope.repository_identity,
            execution_mode: descriptor.execution_mode,
            execution_root: descriptor.execution_root.clone(),
            base_ref: descriptor.base_ref.clone(),
            worktree_id: descriptor.worktree_id.clone(),
            operation: CodeThreadPreparationOperation::Fork,
            source_thread_id: Some(source_thread_id.clone()),
            state: CodeThreadPreparationState::Prepared,
            recovery_thread_baseline: None,
            merge_target_ref,
        };
        preparation.validate()?;

        if index.preparations.len() >= MAX_PREPARATIONS {
            return Err(format!(
                "SchoolX Code binding index reached the {MAX_PREPARATIONS}-preparation limit"
            ));
        }
        if index
            .preparations
            .iter()
            .any(|existing| existing.preparation_id == preparation.preparation_id)
        {
            return Err("SchoolX Code preparation id is already reserved".to_string());
        }
        if index.preparations.iter().any(|existing| {
            existing.is_in_scope(&preparation.scope())
                && existing.operation == CodeThreadPreparationOperation::Fork
                && existing.source_thread_id.as_deref() == Some(source_thread_id.as_str())
        }) {
            return Err(
                "SchoolX Code source thread already has an unfinished fork preparation".to_string(),
            );
        }
        ensure_managed_execution_unreserved(&index, &preparation)?;
        index.preparations.push(preparation.clone());
        index.sort();
        index.validate()?;
        self.save(&index)?;
        Ok(preparation)
    }

    /// List native execution preparations in one exact project scope.
    pub fn list_preparations(
        &self,
        scope: &CodeThreadBindingScope,
    ) -> Result<Vec<CodeThreadPreparation>, String> {
        scope.validate()?;
        Ok(self
            .load()?
            .preparations
            .into_iter()
            .filter(|preparation| preparation.is_in_scope(scope))
            .map(|mut preparation| {
                preparation.recovery_thread_baseline = None;
                preparation.merge_target_ref = None;
                preparation
            })
            .collect())
    }

    /// Atomically mark a preparation as submitted before `thread/start`.
    ///
    /// A `starting` record is deliberately sticky. Any RPC error may be an
    /// ambiguous timeout after Codex created the thread, so repeating start is
    /// forbidden until native recovery discovers the created thread.
    pub(crate) fn claim_preparation_for_start(
        &self,
        scope: &CodeThreadBindingScope,
        preparation_id: &str,
        mut recovery_thread_baseline: Vec<String>,
    ) -> Result<CodeThreadPreparation, String> {
        scope.validate()?;
        validate_preparation_id(preparation_id)?;
        recovery_thread_baseline.sort();
        validate_recovery_baseline(&recovery_thread_baseline)?;
        let mut index = self.load()?;
        let preparation = index
            .preparations
            .iter_mut()
            .find(|preparation| {
                preparation.preparation_id == preparation_id && preparation.is_in_scope(scope)
            })
            .ok_or_else(|| {
                "SchoolX Code preparation was not found in the requested scope".to_string()
            })?;
        if preparation.operation != CodeThreadPreparationOperation::Start {
            return Err(
                "SchoolX Code fork preparation cannot be consumed by thread/start".to_string(),
            );
        }
        if preparation.state == CodeThreadPreparationState::Starting {
            return Err(
                "SchoolX Code preparation already crossed thread/start and requires recovery"
                    .to_string(),
            );
        }
        preparation.state = CodeThreadPreparationState::Starting;
        preparation.recovery_thread_baseline = Some(recovery_thread_baseline);
        let claimed = preparation.clone();
        index.sort();
        index.validate()?;
        self.save(&index)?;
        Ok(claimed)
    }

    /// Atomically mark an exact fork preparation as submitted.
    pub(crate) fn claim_preparation_for_fork(
        &self,
        scope: &CodeThreadBindingScope,
        preparation_id: &str,
        source_thread_id: &str,
        mut recovery_thread_baseline: Vec<String>,
    ) -> Result<CodeThreadPreparation, String> {
        scope.validate()?;
        validate_preparation_id(preparation_id)?;
        validate_identifier("fork source thread", source_thread_id)?;
        recovery_thread_baseline.sort();
        validate_recovery_baseline(&recovery_thread_baseline)?;
        let mut index = self.load()?;
        let preparation = index
            .preparations
            .iter_mut()
            .find(|preparation| {
                preparation.preparation_id == preparation_id && preparation.is_in_scope(scope)
            })
            .ok_or_else(|| {
                "SchoolX Code preparation was not found in the requested scope".to_string()
            })?;
        if preparation.operation != CodeThreadPreparationOperation::Fork
            || preparation.source_thread_id.as_deref() != Some(source_thread_id)
        {
            return Err(
                "SchoolX Code fork preparation does not match its exact source thread".to_string(),
            );
        }
        if preparation.state == CodeThreadPreparationState::Starting {
            return Err(
                "SchoolX Code preparation already crossed thread/fork and requires recovery"
                    .to_string(),
            );
        }
        preparation.state = CodeThreadPreparationState::Starting;
        preparation.recovery_thread_baseline = Some(recovery_thread_baseline);
        let claimed = preparation.clone();
        index.sort();
        index.validate()?;
        self.save(&index)?;
        Ok(claimed)
    }

    /// Restore a claimed preparation only when native transport proved that
    /// no `thread/start` bytes were submitted to Codex.
    ///
    /// Timeouts, partial writes, disconnects, and app-server errors must never
    /// call this method because they may have created a thread.
    pub(crate) fn restore_preparation_after_unsent_start(
        &self,
        claimed: &CodeThreadPreparation,
    ) -> Result<CodeThreadPreparation, String> {
        claimed.validate()?;
        if claimed.state != CodeThreadPreparationState::Starting {
            return Err("SchoolX Code rollback requires the exact claimed preparation".to_string());
        }
        let mut index = self.load()?;
        let preparation = index
            .preparations
            .iter_mut()
            .find(|preparation| preparation.preparation_id == claimed.preparation_id)
            .ok_or_else(|| {
                "SchoolX Code preparation was not found for unsent-start rollback".to_string()
            })?;
        if preparation != claimed {
            return Err(
                "SchoolX Code preparation changed after claim; unsent-start rollback was refused"
                    .to_string(),
            );
        }
        preparation.state = CodeThreadPreparationState::Prepared;
        preparation.recovery_thread_baseline = None;
        let restored = preparation.clone();
        index.sort();
        index.validate()?;
        self.save(&index)?;
        Ok(restored)
    }

    /// Restore the exact fork claim only when no request bytes were admitted.
    pub(crate) fn restore_preparation_after_unsent_fork(
        &self,
        claimed: &CodeThreadPreparation,
    ) -> Result<CodeThreadPreparation, String> {
        if claimed.operation != CodeThreadPreparationOperation::Fork {
            return Err("SchoolX Code unsent-fork rollback requires a fork claim".to_string());
        }
        self.restore_claimed_preparation(claimed, "fork")
    }

    fn restore_claimed_preparation(
        &self,
        claimed: &CodeThreadPreparation,
        operation: &str,
    ) -> Result<CodeThreadPreparation, String> {
        claimed.validate()?;
        if claimed.state != CodeThreadPreparationState::Starting {
            return Err(format!(
                "SchoolX Code unsent-{operation} rollback requires the exact claimed preparation"
            ));
        }
        let mut index = self.load()?;
        let preparation = index
            .preparations
            .iter_mut()
            .find(|preparation| preparation.preparation_id == claimed.preparation_id)
            .ok_or_else(|| {
                format!("SchoolX Code preparation was not found for unsent-{operation} rollback")
            })?;
        if preparation != claimed {
            return Err(format!(
                "SchoolX Code preparation changed after claim; unsent-{operation} rollback was refused"
            ));
        }
        preparation.state = CodeThreadPreparationState::Prepared;
        preparation.recovery_thread_baseline = None;
        let restored = preparation.clone();
        index.sort();
        index.validate()?;
        self.save(&index)?;
        Ok(restored)
    }

    /// Read one still-safe native preparation before filesystem revalidation.
    pub(crate) fn prepared_preparation(
        &self,
        scope: &CodeThreadBindingScope,
        preparation_id: &str,
    ) -> Result<CodeThreadPreparation, String> {
        scope.validate()?;
        validate_preparation_id(preparation_id)?;
        let preparation = self
            .load()?
            .preparations
            .into_iter()
            .find(|preparation| {
                preparation.preparation_id == preparation_id && preparation.is_in_scope(scope)
            })
            .ok_or_else(|| {
                "SchoolX Code preparation was not found in the requested scope".to_string()
            })?;
        if preparation.state != CodeThreadPreparationState::Prepared {
            return Err(
                "SchoolX Code preparation already crossed thread/start and requires recovery"
                    .to_string(),
            );
        }
        Ok(preparation)
    }

    /// Read one exact preparation without weakening its persisted operation state.
    pub(crate) fn preparation(
        &self,
        scope: &CodeThreadBindingScope,
        preparation_id: &str,
    ) -> Result<CodeThreadPreparation, String> {
        scope.validate()?;
        validate_preparation_id(preparation_id)?;
        self.load()?
            .preparations
            .into_iter()
            .find(|preparation| {
                preparation.preparation_id == preparation_id && preparation.is_in_scope(scope)
            })
            .ok_or_else(|| {
                "SchoolX Code preparation was not found in the requested scope".to_string()
            })
    }

    /// Read a `starting` preparation for explicit orphan reconciliation.
    pub(crate) fn starting_preparation(
        &self,
        scope: &CodeThreadBindingScope,
        preparation_id: &str,
    ) -> Result<CodeThreadPreparation, String> {
        scope.validate()?;
        validate_preparation_id(preparation_id)?;
        let preparation = self
            .load()?
            .preparations
            .into_iter()
            .find(|preparation| {
                preparation.preparation_id == preparation_id && preparation.is_in_scope(scope)
            })
            .ok_or_else(|| {
                "SchoolX Code preparation was not found in the requested scope".to_string()
            })?;
        if preparation.state != CodeThreadPreparationState::Starting {
            return Err(
                "SchoolX Code preparation has not crossed thread/start and cannot be recovered"
                    .to_string(),
            );
        }
        Ok(preparation)
    }

    /// Atomically replace one `starting` preparation with its final binding.
    pub(crate) fn commit_preparation_binding(
        &self,
        scope: &CodeThreadBindingScope,
        preparation_id: &str,
        codex_thread_id: &str,
    ) -> Result<CodeThreadBinding, String> {
        scope.validate()?;
        validate_preparation_id(preparation_id)?;
        validate_identifier("Codex thread", codex_thread_id)?;
        let mut index = self.load()?;
        let preparation_index = index
            .preparations
            .iter()
            .position(|preparation| {
                preparation.preparation_id == preparation_id && preparation.is_in_scope(scope)
            })
            .ok_or_else(|| {
                "SchoolX Code preparation was not found in the requested scope".to_string()
            })?;
        let preparation = index.preparations[preparation_index].clone();
        if preparation.state != CodeThreadPreparationState::Starting {
            return Err(
                "SchoolX Code preparation must be claimed before binding a thread".to_string(),
            );
        }
        if index
            .bindings
            .iter()
            .any(|binding| binding.codex_thread_id == codex_thread_id)
        {
            return Err(format!(
                "Codex thread {codex_thread_id} is already bound to a SchoolX Code execution root"
            ));
        }
        if removal::reserves_thread_id(&index, codex_thread_id) {
            return Err(format!(
                "Codex thread {codex_thread_id} is permanently reserved by SchoolX Code removal state"
            ));
        }
        if preparation.operation == CodeThreadPreparationOperation::Fork {
            let source_thread_id = preparation.source_thread_id.as_deref().ok_or_else(|| {
                "SchoolX Code fork preparation is missing its source thread".to_string()
            })?;
            if source_thread_id == codex_thread_id {
                return Err(
                    "Codex fork returned the source thread instead of a new thread".to_string(),
                );
            }
            let source = index
                .bindings
                .iter()
                .find(|binding| {
                    binding.codex_thread_id == source_thread_id && binding.is_in_scope(scope)
                })
                .ok_or_else(|| {
                    "SchoolX Code fork source binding disappeared before destination commit"
                        .to_string()
                })?;
            if source.execution_mode != CodeExecutionMode::Worktree {
                return Err(
                    "SchoolX Code fork source is not a managed worktree binding".to_string()
                );
            }
            if source.execution_root == preparation.execution_root
                || source.worktree_id == preparation.worktree_id
            {
                return Err(
                    "SchoolX Code fork source and destination cannot share a managed worktree"
                        .to_string(),
                );
            }
        }

        let merge_target_ref = preparation.merge_target_ref.clone();
        let binding = CodeThreadBinding {
            community_id: preparation.community_id,
            project_dtag: preparation.project_dtag,
            repository_identity: preparation.repository_identity,
            codex_thread_id: codex_thread_id.to_string(),
            execution_mode: preparation.execution_mode,
            execution_root: preparation.execution_root,
            base_ref: preparation.base_ref,
            worktree_id: preparation.worktree_id,
        };
        binding.validate()?;
        index.preparations.remove(preparation_index);
        index.bindings.push(binding.clone());
        lifecycle::insert_active_lifecycle(&mut index.lifecycles, &binding)?;
        if let Some(target_ref) = merge_target_ref {
            index
                .merge_targets
                .push(CodeWorktreeMergeTarget::for_binding(&binding, target_ref)?);
        }
        index.sort();
        index.validate()?;
        self.save(&index)?;
        Ok(binding)
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

    fn save(&self, index: &CodeThreadBindingIndex) -> Result<(), String> {
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

fn validate_identifier(label: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_IDENTIFIER_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(format!(
            "SchoolX Code {label} must be a trimmed, non-control string between 1 and {MAX_IDENTIFIER_BYTES} bytes"
        ));
    }
    Ok(())
}

fn validate_recovery_baseline(thread_ids: &[String]) -> Result<(), String> {
    if thread_ids.len() > MAX_RECOVERY_BASELINE_THREADS {
        return Err(format!(
            "SchoolX Code recovery baseline exceeds the {MAX_RECOVERY_BASELINE_THREADS}-thread limit"
        ));
    }
    for thread_id in thread_ids {
        validate_identifier("recovery baseline thread", thread_id)?;
    }
    if thread_ids
        .windows(2)
        .any(|pair| pair[0].as_str() >= pair[1].as_str())
    {
        return Err(
            "SchoolX Code recovery baseline must contain unique sorted thread ids".to_string(),
        );
    }
    Ok(())
}

fn validate_sha256(label: &str, value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "SchoolX Code {label} must be 64 lowercase hexadecimal characters"
        ));
    }
    Ok(())
}

fn validate_commit_id(value: &str) -> Result<(), String> {
    if !matches!(value.len(), 40 | 64)
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("SchoolX Code base ref must be a resolved lowercase Git commit id".to_string());
    }
    Ok(())
}

/// Validate the only ref namespace that may become native merge authority.
/// This is intentionally narrower than a general Git revision parser.
pub(crate) fn validate_direct_local_branch_ref(value: &str) -> Result<(), String> {
    let Some(branch) = value.strip_prefix("refs/heads/") else {
        return Err(
            "SchoolX Code merge target must be a fully qualified local branch ref".to_string(),
        );
    };
    if branch.is_empty()
        || value.len() > MAX_IDENTIFIER_BYTES
        || value.ends_with('/')
        || value.ends_with('.')
        || value.contains("..")
        || value.contains("@{")
        || value.contains("//")
        || value.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || matches!(character, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
        || branch.split('/').any(|component| {
            component.is_empty()
                || component == "."
                || component == ".."
                || component == "@"
                || component.starts_with('.')
                || component.ends_with(".lock")
        })
    {
        return Err("SchoolX Code merge target is not a safe direct local branch ref".to_string());
    }
    Ok(())
}

fn validate_preparation_id(value: &str) -> Result<(), String> {
    let parsed = uuid::Uuid::parse_str(value)
        .map_err(|error| format!("SchoolX Code preparation id is not a UUID: {error}"))?;
    if parsed.hyphenated().to_string() != value {
        return Err(
            "SchoolX Code preparation id must be a canonical lowercase hyphenated UUID".to_string(),
        );
    }
    Ok(())
}

fn validate_worktree_id(value: &str) -> Result<(), String> {
    if value != value.to_ascii_lowercase() {
        return Err("SchoolX Code worktree id must be lowercase".to_string());
    }
    let parsed = uuid::Uuid::parse_str(value)
        .map_err(|error| format!("SchoolX Code worktree id is not a UUID: {error}"))?;
    let hyphenated = parsed.hyphenated().to_string();
    if value != hyphenated {
        return Err(
            "SchoolX Code worktree id must be a canonical lowercase hyphenated UUID".to_string(),
        );
    }
    Ok(())
}

fn ensure_managed_execution_unreserved(
    index: &CodeThreadBindingIndex,
    preparation: &CodeThreadPreparation,
) -> Result<(), String> {
    if removal::reserves_execution(
        index,
        preparation.worktree_id.as_deref(),
        &preparation.execution_root,
    ) {
        return Err(
            "SchoolX Code execution identity is permanently reserved by removal state".to_string(),
        );
    }
    if preparation.execution_mode == CodeExecutionMode::Local {
        return Ok(());
    }
    let worktree_id = preparation.worktree_id.as_deref().ok_or_else(|| {
        "SchoolX Code worktree preparation is missing its worktree id".to_string()
    })?;
    let bound = index.bindings.iter().any(|binding| {
        binding.execution_mode == CodeExecutionMode::Worktree
            && (binding.worktree_id.as_deref() == Some(worktree_id)
                || binding.execution_root == preparation.execution_root)
    });
    let prepared = index.preparations.iter().any(|existing| {
        existing.execution_mode == CodeExecutionMode::Worktree
            && (existing.worktree_id.as_deref() == Some(worktree_id)
                || existing.execution_root == preparation.execution_root)
    });
    if bound || prepared {
        return Err(format!(
            "managed worktree {worktree_id} is already reserved by SchoolX Code"
        ));
    }
    Ok(())
}

/// Validate only the durable representation of an execution root.
///
/// Loading the index must not depend on every checkout still existing. Live
/// filesystem and Git checks happen when a binding or preparation is used, so
/// one unavailable root cannot hide otherwise healthy scopes.
fn validate_execution_root(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > MAX_EXECUTION_ROOT_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(format!(
            "SchoolX Code execution root must be between 1 and {MAX_EXECUTION_ROOT_BYTES} bytes"
        ));
    }
    let path = Path::new(value);
    if !path.is_absolute() {
        return Err("SchoolX Code execution root must be absolute".to_string());
    }
    if path
        .components()
        .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        return Err("SchoolX Code execution root must not contain `.` or `..`".to_string());
    }

    Ok(())
}

fn validate_live_execution_root(value: &str) -> Result<(), String> {
    validate_execution_root(value)?;
    let path = Path::new(value);
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect SchoolX Code execution root: {error}"))?;
    if metadata.file_type().is_symlink() {
        return Err("SchoolX Code execution root cannot be a symlink".to_string());
    }
    if !metadata.is_dir() {
        return Err("SchoolX Code execution root is not a directory".to_string());
    }

    let canonical = path
        .canonicalize()
        .map_err(|error| format!("failed to resolve SchoolX Code execution root: {error}"))?;
    let canonical = canonical.to_str().ok_or_else(|| {
        "SchoolX Code execution root is not valid Unicode after canonicalization".to_string()
    })?;
    if canonical != value {
        return Err("SchoolX Code execution root is not canonical".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn repository_identity(marker: char) -> String {
        marker.to_string().repeat(64)
    }

    fn scope(community: &str, project: &str, marker: char) -> CodeThreadBindingScope {
        CodeThreadBindingScope {
            community_id: community.to_string(),
            project_dtag: project.to_string(),
            repository_identity: repository_identity(marker),
        }
    }

    fn binding(
        root: &Path,
        scope: CodeThreadBindingScope,
        thread_id: &str,
        mode: CodeExecutionMode,
        worktree_id: Option<&str>,
    ) -> CodeThreadBinding {
        CodeThreadBinding {
            community_id: scope.community_id,
            project_dtag: scope.project_dtag,
            repository_identity: scope.repository_identity,
            codex_thread_id: thread_id.to_string(),
            execution_mode: mode,
            execution_root: root.to_string_lossy().into_owned(),
            base_ref: "0123456789abcdef0123456789abcdef01234567".to_string(),
            worktree_id: worktree_id.map(str::to_string),
        }
    }

    fn local_descriptor(root: &Path, marker: char) -> CodeWorktreeDescriptor {
        CodeWorktreeDescriptor {
            execution_mode: CodeExecutionMode::Local,
            repository_identity: repository_identity(marker),
            execution_root: root.to_string_lossy().into_owned(),
            base_ref: "0123456789abcdef0123456789abcdef01234567".to_string(),
            worktree_id: None,
        }
    }

    fn store() -> (tempfile::TempDir, CodeThreadBindingStore) {
        let directory = tempfile::tempdir().expect("temp app data");
        let store = CodeThreadBindingStore::for_app_data(directory.path()).expect("binding store");
        (directory, store)
    }

    #[test]
    fn missing_store_loads_as_empty_current_version() {
        let (_directory, store) = store();
        let index = store.load().expect("empty index");

        assert_eq!(index.version, CODE_THREAD_BINDING_SCHEMA_VERSION);
        assert!(index.bindings.is_empty());
        assert!(!store.store_path().exists());
    }

    #[test]
    fn preparation_journal_reloads_scopes_claims_and_commits_atomically() {
        let (directory, store) = store();
        let root = directory.path().join("local-preparation");
        fs::create_dir(&root).expect("local preparation root");
        let root = root.canonicalize().expect("canonical preparation root");
        let owner = scope("community-a", "project-a", 'a');
        let preparation_id = "67f11a1d-0274-4d40-9b0c-e406e51c64fb";
        let descriptor = local_descriptor(&root, 'a');

        let prepared = store
            .create_preparation(preparation_id.to_string(), owner.clone(), &descriptor)
            .expect("create preparation");
        assert_eq!(prepared.state, CodeThreadPreparationState::Prepared);

        let reloaded = CodeThreadBindingStore::for_app_data(directory.path())
            .expect("reopen preparation store");
        assert_eq!(
            reloaded
                .list_preparations(&owner)
                .expect("owner preparations"),
            vec![prepared.clone()]
        );
        assert!(reloaded
            .list_preparations(&scope("community-b", "project-a", 'a'))
            .expect("other-scope preparations")
            .is_empty());

        let claimed = reloaded
            .claim_preparation_for_start(&owner, preparation_id, vec!["thread-before".to_string()])
            .expect("claim preparation");
        assert_eq!(claimed.state, CodeThreadPreparationState::Starting);
        assert_eq!(
            claimed.recovery_thread_baseline,
            Some(vec!["thread-before".to_string()])
        );
        assert!(reloaded
            .claim_preparation_for_start(&owner, preparation_id, Vec::new())
            .is_err());

        let after_restart = CodeThreadBindingStore::for_app_data(directory.path())
            .expect("restart preparation store");
        assert_eq!(
            after_restart
                .starting_preparation(&owner, preparation_id)
                .expect("durable starting preparation")
                .recovery_thread_baseline,
            Some(vec!["thread-before".to_string()])
        );
        let binding = after_restart
            .commit_preparation_binding(&owner, preparation_id, "thread-recovered")
            .expect("commit recovered binding");
        assert_eq!(binding.codex_thread_id, "thread-recovered");
        let final_index = after_restart.load().expect("final binding index");
        assert!(final_index.preparations.is_empty());
        assert_eq!(final_index.bindings, vec![binding]);
    }

    #[test]
    fn exact_unsent_start_snapshot_restores_and_reloads_as_prepared() {
        let (directory, store) = store();
        let root = directory.path().join("rollback-preparation");
        fs::create_dir(&root).expect("local preparation root");
        let root = root.canonicalize().expect("canonical preparation root");
        let owner = scope("community-a", "project-a", 'a');
        let preparation_id = "67f11a1d-0274-4d40-9b0c-e406e51c64fb";
        store
            .create_preparation(
                preparation_id.to_string(),
                owner.clone(),
                &local_descriptor(&root, 'a'),
            )
            .expect("create preparation");
        let claimed = store
            .claim_preparation_for_start(
                &owner,
                preparation_id,
                vec!["thread-z".to_string(), "thread-a".to_string()],
            )
            .expect("claim preparation");
        assert_eq!(
            claimed.recovery_thread_baseline,
            Some(vec!["thread-a".to_string(), "thread-z".to_string()])
        );

        let after_claim = CodeThreadBindingStore::for_app_data(directory.path())
            .expect("reopen claimed preparation store");
        assert_eq!(
            after_claim
                .starting_preparation(&owner, preparation_id)
                .expect("durable claimed preparation"),
            claimed
        );
        let restored = after_claim
            .restore_preparation_after_unsent_start(&claimed)
            .expect("restore definitely-unsent preparation");
        assert_eq!(restored.state, CodeThreadPreparationState::Prepared);
        assert_eq!(restored.recovery_thread_baseline, None);

        let after_restore = CodeThreadBindingStore::for_app_data(directory.path())
            .expect("reopen restored preparation store");
        assert_eq!(
            after_restore
                .prepared_preparation(&owner, preparation_id)
                .expect("durable restored preparation"),
            restored
        );
        assert!(after_restore
            .starting_preparation(&owner, preparation_id)
            .is_err());
    }

    #[test]
    fn forged_unsent_start_snapshot_is_rejected_without_changing_starting_record() {
        let (directory, store) = store();
        let root = directory.path().join("forged-rollback-preparation");
        fs::create_dir(&root).expect("local preparation root");
        let root = root.canonicalize().expect("canonical preparation root");
        let owner = scope("community-a", "project-a", 'a');
        let preparation_id = "67f11a1d-0274-4d40-9b0c-e406e51c64fb";
        store
            .create_preparation(
                preparation_id.to_string(),
                owner.clone(),
                &local_descriptor(&root, 'a'),
            )
            .expect("create preparation");
        let claimed = store
            .claim_preparation_for_start(&owner, preparation_id, vec!["thread-before".to_string()])
            .expect("claim preparation");
        let mut forged = claimed.clone();
        forged.recovery_thread_baseline = Some(vec!["thread-forged".to_string()]);

        let error = store
            .restore_preparation_after_unsent_start(&forged)
            .expect_err("forged claimed snapshot must not roll back");
        assert!(
            error.contains("changed after claim"),
            "unexpected rollback error: {error}"
        );

        let reloaded = CodeThreadBindingStore::for_app_data(directory.path())
            .expect("reopen rejected rollback store");
        assert_eq!(
            reloaded
                .starting_preparation(&owner, preparation_id)
                .expect("unchanged starting preparation"),
            claimed
        );
        assert!(reloaded
            .prepared_preparation(&owner, preparation_id)
            .is_err());
    }

    #[test]
    fn managed_preparation_reserves_its_worktree_before_thread_start() {
        let (directory, store) = store();
        let owner = scope("community", "project", 'a');
        let root = directory
            .path()
            .canonicalize()
            .expect("canonical app data")
            .join("managed-preparation");
        fs::create_dir(&root).expect("managed preparation root");
        let descriptor = CodeWorktreeDescriptor {
            execution_mode: CodeExecutionMode::Worktree,
            repository_identity: owner.repository_identity.clone(),
            execution_root: root.to_string_lossy().into_owned(),
            base_ref: "0123456789abcdef0123456789abcdef01234567".to_string(),
            worktree_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
        };
        store
            .create_preparation(
                "67f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
                owner.clone(),
                &descriptor,
            )
            .expect("managed preparation");

        assert!(store
            .create_preparation(
                "77f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
                owner.clone(),
                &descriptor,
            )
            .is_err());
        assert!(store
            .upsert(binding(
                &root,
                owner,
                "thread-duplicate",
                CodeExecutionMode::Worktree,
                Some("11111111-1111-4111-8111-111111111111"),
            ))
            .is_err());
    }

    #[test]
    fn fork_preparation_binds_one_managed_source_and_rolls_back_exactly() {
        let (directory, store) = store();
        let owner = scope("community", "project", 'a');
        let source_root = directory.path().join("source-worktree");
        let destination_root = directory.path().join("destination-worktree");
        fs::create_dir(&source_root).expect("source root");
        fs::create_dir(&destination_root).expect("destination root");
        let source_root = source_root.canonicalize().expect("canonical source root");
        let destination_root = destination_root
            .canonicalize()
            .expect("canonical destination root");
        store
            .upsert(binding(
                &source_root,
                owner.clone(),
                "thread-source",
                CodeExecutionMode::Worktree,
                Some("11111111-1111-4111-8111-111111111111"),
            ))
            .expect("source binding");
        let descriptor = CodeWorktreeDescriptor {
            execution_mode: CodeExecutionMode::Worktree,
            repository_identity: owner.repository_identity.clone(),
            execution_root: destination_root.to_string_lossy().into_owned(),
            base_ref: "0123456789abcdef0123456789abcdef01234567".to_string(),
            worktree_id: Some("22222222-2222-4222-8222-222222222222".to_string()),
        };
        let preparation_id = "67f11a1d-0274-4d40-9b0c-e406e51c64fb";
        let prepared = store
            .create_fork_preparation(
                preparation_id.to_string(),
                owner.clone(),
                "thread-source".to_string(),
                &descriptor,
            )
            .expect("fork preparation");
        assert_eq!(prepared.operation, CodeThreadPreparationOperation::Fork);
        assert_eq!(prepared.source_thread_id.as_deref(), Some("thread-source"));
        assert!(prepared.merge_target_ref.is_none());
        assert!(store
            .ensure_fork_source_available(&owner, "thread-source")
            .is_err());
        assert!(store
            .create_fork_preparation(
                "77f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
                owner.clone(),
                "thread-source".to_string(),
                &CodeWorktreeDescriptor {
                    worktree_id: Some("33333333-3333-4333-8333-333333333333".to_string()),
                    ..descriptor.clone()
                },
            )
            .is_err());

        let claimed = store
            .claim_preparation_for_fork(
                &owner,
                preparation_id,
                "thread-source",
                vec!["thread-z".to_string(), "thread-a".to_string()],
            )
            .expect("claim fork");
        assert_eq!(
            claimed.recovery_thread_baseline,
            Some(vec!["thread-a".to_string(), "thread-z".to_string()])
        );
        let restored = store
            .restore_preparation_after_unsent_fork(&claimed)
            .expect("restore exact fork claim");
        assert_eq!(restored.state, CodeThreadPreparationState::Prepared);
        assert!(restored.recovery_thread_baseline.is_none());

        let claimed = store
            .claim_preparation_for_fork(&owner, preparation_id, "thread-source", Vec::new())
            .expect("reclaim fork");
        assert_eq!(claimed.state, CodeThreadPreparationState::Starting);
        let child = store
            .commit_preparation_binding(&owner, preparation_id, "thread-child")
            .expect("commit fork child");
        assert_eq!(child.codex_thread_id, "thread-child");
        assert_eq!(child.execution_root, descriptor.execution_root);
        let final_index = store.load().expect("final fork index");
        assert!(final_index.preparations.is_empty());
        assert_eq!(final_index.bindings.len(), 2);

        let mut malformed = final_index;
        malformed.preparations.push(CodeThreadPreparation {
            preparation_id: "87f11a1d-0274-4d40-9b0c-e406e51c64fb".to_string(),
            community_id: owner.community_id,
            project_dtag: owner.project_dtag,
            repository_identity: owner.repository_identity,
            execution_mode: CodeExecutionMode::Worktree,
            execution_root: destination_root.to_string_lossy().into_owned(),
            base_ref: descriptor.base_ref,
            worktree_id: Some("44444444-4444-4444-8444-444444444444".to_string()),
            operation: CodeThreadPreparationOperation::Fork,
            source_thread_id: Some("thread-missing".to_string()),
            state: CodeThreadPreparationState::Prepared,
            recovery_thread_baseline: None,
            merge_target_ref: None,
        });
        assert!(malformed.validate().is_err());
    }

    #[test]
    fn merge_target_authority_is_native_only_and_moves_or_copies_atomically() {
        let (directory, store) = store();
        let owner = scope("community", "project", 'a');
        let source_root = directory.path().join("authority-source");
        let fork_root = directory.path().join("authority-fork");
        fs::create_dir(&source_root).expect("source root");
        fs::create_dir(&fork_root).expect("fork root");
        let source_root = source_root.canonicalize().expect("canonical source root");
        let fork_root = fork_root.canonicalize().expect("canonical fork root");
        let source_descriptor = CodeWorktreeDescriptor {
            execution_mode: CodeExecutionMode::Worktree,
            repository_identity: owner.repository_identity.clone(),
            execution_root: source_root.to_string_lossy().into_owned(),
            base_ref: "0123456789abcdef0123456789abcdef01234567".to_string(),
            worktree_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
        };
        let source_preparation = "11111111-1111-4111-8111-111111111112";
        let prepared = store
            .create_preparation_with_merge_target(
                source_preparation.to_string(),
                owner.clone(),
                &source_descriptor,
                Some("refs/heads/main".to_string()),
            )
            .expect("native-authorized preparation");
        assert_eq!(
            prepared.merge_target_ref.as_deref(),
            Some("refs/heads/main")
        );
        let public = store
            .list_preparations(&owner)
            .expect("public preparations");
        assert!(public[0].merge_target_ref.is_none());
        assert!(!serde_json::to_value(&public[0])
            .expect("serialize public preparation")
            .as_object()
            .expect("preparation object")
            .contains_key("mergeTargetRef"));

        let claimed = store
            .claim_preparation_for_start(&owner, source_preparation, Vec::new())
            .expect("claim source");
        assert_eq!(claimed.merge_target_ref.as_deref(), Some("refs/heads/main"));
        let restored = store
            .restore_preparation_after_unsent_start(&claimed)
            .expect("restore source");
        assert_eq!(
            restored.merge_target_ref.as_deref(),
            Some("refs/heads/main")
        );
        store
            .claim_preparation_for_start(&owner, source_preparation, Vec::new())
            .expect("reclaim source");
        let source_binding = store
            .commit_preparation_binding(&owner, source_preparation, "thread-source")
            .expect("commit source");
        assert_eq!(
            serde_json::to_value(&source_binding)
                .expect("serialize binding")
                .as_object()
                .map(|object| object.len()),
            Some(8)
        );
        let source_lookup = CodeThreadBindingLookupInput {
            scope: owner.clone(),
            codex_thread_id: "thread-source".to_string(),
        };
        let (_, target_ref) = store
            .binding_merge_authority(&source_lookup)
            .expect("binding authority")
            .expect("source binding");
        assert_eq!(target_ref.as_deref(), Some("refs/heads/main"));
        let valid_index = store.load().expect("authority index");
        assert_eq!(valid_index.merge_targets.len(), 1);
        let persisted = fs::read(store.store_path()).expect("read authority index");
        let persisted: serde_json::Value =
            serde_json::from_slice(&persisted).expect("parse authority index");
        assert_eq!(persisted["version"], serde_json::json!(4));
        assert_eq!(persisted["removals"], serde_json::json!([]));
        let top_level_keys = persisted
            .as_object()
            .expect("binding index object")
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            top_level_keys,
            [
                "bindings",
                "lifecycles",
                "mergeTargets",
                "preparations",
                "removals",
                "version",
            ]
            .into_iter()
            .collect::<BTreeSet<_>>()
        );
        let target_keys = persisted["mergeTargets"][0]
            .as_object()
            .expect("merge-target object")
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert_eq!(
            target_keys,
            [
                "codexThreadId",
                "communityId",
                "projectDtag",
                "repositoryIdentity",
                "targetRef",
                "worktreeId",
            ]
            .into_iter()
            .collect::<BTreeSet<_>>()
        );
        let mut duplicate = valid_index.clone();
        let duplicate_authority = duplicate.merge_targets[0].clone();
        duplicate.merge_targets.push(duplicate_authority);
        assert!(duplicate.validate().is_err());
        let mut orphan = valid_index.clone();
        orphan.merge_targets[0].codex_thread_id = "thread-orphan".to_string();
        assert!(orphan.validate().is_err());
        let mut wrong_worktree = valid_index;
        wrong_worktree.merge_targets[0].worktree_id =
            "33333333-3333-4333-8333-333333333333".to_string();
        assert!(wrong_worktree.validate().is_err());

        let fork_descriptor = CodeWorktreeDescriptor {
            execution_mode: CodeExecutionMode::Worktree,
            repository_identity: owner.repository_identity.clone(),
            execution_root: fork_root.to_string_lossy().into_owned(),
            base_ref: source_descriptor.base_ref,
            worktree_id: Some("22222222-2222-4222-8222-222222222222".to_string()),
        };
        let fork = store
            .create_fork_preparation(
                "22222222-2222-4222-8222-222222222223".to_string(),
                owner,
                "thread-source".to_string(),
                &fork_descriptor,
            )
            .expect("fork preparation");
        assert_eq!(fork.merge_target_ref.as_deref(), Some("refs/heads/main"));
    }

    #[test]
    fn merge_target_ref_validator_rejects_non_local_and_revision_syntax() {
        assert!(validate_direct_local_branch_ref("refs/heads/main").is_ok());
        assert!(validate_direct_local_branch_ref("refs/heads/team/topic").is_ok());
        for invalid in [
            "HEAD",
            "main",
            "refs/tags/main",
            "refs/remotes/origin/main",
            "refs/heads/main~1",
            "refs/heads/main@{1}",
            "refs/heads/.hidden",
            "refs/heads/main.lock",
            "refs/heads/team//topic",
        ] {
            assert!(
                validate_direct_local_branch_ref(invalid).is_err(),
                "unexpected valid merge target: {invalid}"
            );
        }
    }

    #[test]
    fn version_one_without_preparations_remains_readable() {
        let (_directory, store) = store();
        fs::write(store.store_path(), r#"{"version":1,"bindings":[]}"#)
            .expect("legacy version-one fixture");

        let index = store.load().expect("legacy version-one index");
        assert!(index.bindings.is_empty());
        assert!(index.preparations.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn code_directory_is_owner_only_without_changing_app_data_mode() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().expect("temp app data");
        let code_dir = directory.path().join(CODE_STORE_DIRECTORY);
        fs::create_dir(&code_dir).expect("existing code directory");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755))
            .expect("app-data permissions");
        fs::set_permissions(&code_dir, fs::Permissions::from_mode(0o777))
            .expect("code permissions");

        CodeThreadBindingStore::for_app_data(directory.path()).expect("binding store");

        let app_mode = fs::metadata(directory.path())
            .expect("app-data metadata")
            .permissions()
            .mode()
            & 0o777;
        let code_mode = fs::metadata(&code_dir)
            .expect("code metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(app_mode, 0o755);
        assert_eq!(code_mode, 0o700);
    }

    #[test]
    fn round_trip_reload_is_deterministic_and_owner_only() {
        let (directory, store) = store();
        let root_a = directory.path().join("root-a");
        let root_b = directory.path().join("root-b");
        fs::create_dir(&root_a).expect("managed root");
        fs::create_dir(&root_b).expect("local root");
        let root_a = root_a.canonicalize().expect("canonical managed root");
        let root_b = root_b.canonicalize().expect("canonical local root");
        let later = binding(
            &root_b,
            scope("community-b", "project", 'b'),
            "thread-b",
            CodeExecutionMode::Local,
            None,
        );
        let earlier = binding(
            &root_a,
            scope("community-a", "project", 'a'),
            "thread-a",
            CodeExecutionMode::Worktree,
            Some("11111111-1111-4111-8111-111111111111"),
        );

        store.upsert(later.clone()).expect("insert later");
        store.upsert(earlier.clone()).expect("insert earlier");
        let raw = fs::read_to_string(store.store_path()).expect("stored JSON");
        assert!(raw.find("thread-a").expect("thread-a") < raw.find("thread-b").expect("thread-b"));

        let reloaded = CodeThreadBindingStore::for_app_data(directory.path())
            .expect("reopened store")
            .load()
            .expect("reloaded index");
        assert_eq!(reloaded.bindings, vec![earlier, later]);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(store.store_path())
                .expect("store metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn exact_upsert_is_idempotent() {
        let (directory, store) = store();
        let root = directory.path().join("worktree");
        fs::create_dir(&root).expect("managed root");
        let root = root.canonicalize().expect("canonical managed root");
        let record = binding(
            &root,
            scope("community", "project", 'a'),
            "thread-1",
            CodeExecutionMode::Worktree,
            Some("11111111-1111-4111-8111-111111111111"),
        );

        store.upsert(record.clone()).expect("first upsert");
        store.upsert(record.clone()).expect("idempotent upsert");

        assert_eq!(store.load().expect("index").bindings, vec![record]);
    }

    #[test]
    fn list_and_lookup_are_scope_isolated() {
        let (directory, store) = store();
        let wanted_scope = scope("community-a", "project", 'a');
        for name in ["wanted", "other-community", "other-repository"] {
            fs::create_dir(directory.path().join(name)).expect("local scope root");
        }
        let wanted_root = directory
            .path()
            .join("wanted")
            .canonicalize()
            .expect("canonical wanted root");
        let other_community_root = directory
            .path()
            .join("other-community")
            .canonicalize()
            .expect("canonical other-community root");
        let other_repository_root = directory
            .path()
            .join("other-repository")
            .canonicalize()
            .expect("canonical other-repository root");
        let wanted = binding(
            &wanted_root,
            wanted_scope.clone(),
            "thread-wanted",
            CodeExecutionMode::Local,
            None,
        );
        let other_community = binding(
            &other_community_root,
            scope("community-b", "project", 'a'),
            "thread-other-community",
            CodeExecutionMode::Local,
            None,
        );
        let other_repository = binding(
            &other_repository_root,
            scope("community-a", "project", 'b'),
            "thread-other-repository",
            CodeExecutionMode::Local,
            None,
        );
        for record in [&wanted, &other_community, &other_repository] {
            store.upsert(record.clone()).expect("scope fixture");
        }
        fs::remove_dir(&other_community_root).expect("remove unrelated local checkout");

        assert_eq!(
            store.list(&wanted_scope).expect("scoped list"),
            vec![wanted.clone()]
        );
        assert_eq!(
            store
                .lookup(&CodeThreadBindingLookupInput {
                    scope: wanted_scope,
                    codex_thread_id: wanted.codex_thread_id.clone(),
                })
                .expect("matching lookup"),
            Some(wanted.clone())
        );
        assert!(store
            .lookup(&CodeThreadBindingLookupInput {
                scope: scope("community-b", "project", 'a'),
                codex_thread_id: wanted.codex_thread_id,
            })
            .expect("isolated lookup")
            .is_none());
    }

    #[test]
    fn same_thread_cannot_be_rebound() {
        let (directory, store) = store();
        fs::create_dir(directory.path().join("first")).expect("first local root");
        fs::create_dir(directory.path().join("second")).expect("second local root");
        let first_root = directory
            .path()
            .join("first")
            .canonicalize()
            .expect("canonical first root");
        let second_root = directory
            .path()
            .join("second")
            .canonicalize()
            .expect("canonical second root");
        let first = binding(
            &first_root,
            scope("community-a", "project", 'a'),
            "thread-1",
            CodeExecutionMode::Local,
            None,
        );
        let conflicting = binding(
            &second_root,
            scope("community-b", "project", 'b'),
            "thread-1",
            CodeExecutionMode::Local,
            None,
        );
        store.upsert(first.clone()).expect("first binding");

        assert!(store.upsert(conflicting).is_err());
        assert_eq!(store.load().expect("preserved index").bindings, vec![first]);
    }

    #[test]
    fn managed_worktree_cannot_bind_two_threads() {
        let (directory, store) = store();
        let root = directory.path().join("managed-root");
        fs::create_dir(&root).expect("managed root");
        let root = root.canonicalize().expect("canonical managed root");
        let first = binding(
            &root,
            scope("community-a", "project", 'a'),
            "thread-1",
            CodeExecutionMode::Worktree,
            Some("11111111-1111-4111-8111-111111111111"),
        );
        let same_id = binding(
            &directory.path().join("another-root"),
            scope("community-b", "project", 'b'),
            "thread-2",
            CodeExecutionMode::Worktree,
            Some("11111111-1111-4111-8111-111111111111"),
        );
        let same_root = binding(
            &root,
            scope("community-c", "project", 'c'),
            "thread-3",
            CodeExecutionMode::Worktree,
            Some("33333333-3333-4333-8333-333333333333"),
        );
        store.upsert(first.clone()).expect("first binding");

        assert!(store.upsert(same_id).is_err());
        assert!(store.upsert(same_root).is_err());
        assert_eq!(store.load().expect("preserved index").bindings, vec![first]);
    }

    #[test]
    fn local_checkout_can_have_multiple_threads() {
        let (directory, store) = store();
        let root = directory.path().join("local-checkout");
        fs::create_dir(&root).expect("local checkout");
        let root = root.canonicalize().expect("canonical local checkout");
        let first = binding(
            &root,
            scope("community", "project", 'a'),
            "thread-1",
            CodeExecutionMode::Local,
            None,
        );
        let second = binding(
            &root,
            scope("community", "project", 'a'),
            "thread-2",
            CodeExecutionMode::Local,
            None,
        );

        store.upsert(first).expect("first local binding");
        store.upsert(second).expect("second local binding");
        assert_eq!(store.load().expect("local bindings").bindings.len(), 2);
    }

    #[test]
    fn availability_precheck_is_global_for_managed_worktrees() {
        let (directory, store) = store();
        let root = directory.path().join("managed-root");
        fs::create_dir(&root).expect("managed root");
        fs::create_dir(directory.path().join("different-root")).expect("different managed root");
        fs::create_dir(directory.path().join("free-root")).expect("free managed root");
        let root = root.canonicalize().expect("canonical managed root");
        let different_root = directory
            .path()
            .join("different-root")
            .canonicalize()
            .expect("canonical different managed root");
        let free_root = directory
            .path()
            .join("free-root")
            .canonicalize()
            .expect("canonical free managed root");
        store
            .upsert(binding(
                &root,
                scope("community-a", "project-a", 'a'),
                "thread-1",
                CodeExecutionMode::Worktree,
                Some("11111111-1111-4111-8111-111111111111"),
            ))
            .expect("managed fixture");

        assert!(store
            .ensure_execution_available(&CodeExecutionAvailabilityInput {
                execution_mode: CodeExecutionMode::Worktree,
                execution_root: different_root.to_string_lossy().into_owned(),
                worktree_id: Some("11111111-1111-4111-8111-111111111111".to_string()),
            })
            .is_err());
        assert!(store
            .ensure_execution_available(&CodeExecutionAvailabilityInput {
                execution_mode: CodeExecutionMode::Worktree,
                execution_root: root.to_string_lossy().into_owned(),
                worktree_id: Some("22222222-2222-4222-8222-222222222222".to_string()),
            })
            .is_err());
        assert!(store
            .ensure_execution_available(&CodeExecutionAvailabilityInput {
                execution_mode: CodeExecutionMode::Worktree,
                execution_root: free_root.to_string_lossy().into_owned(),
                worktree_id: Some("33333333-3333-4333-8333-333333333333".to_string()),
            })
            .is_ok());
    }

    #[test]
    fn availability_precheck_allows_shared_local_checkout() {
        let (directory, store) = store();
        let root = directory.path().join("local-root");
        fs::create_dir(&root).expect("local root");
        let root = root.canonicalize().expect("canonical local root");
        store
            .upsert(binding(
                &root,
                scope("community", "project", 'a'),
                "thread-1",
                CodeExecutionMode::Local,
                None,
            ))
            .expect("local fixture");

        store
            .ensure_execution_available(&CodeExecutionAvailabilityInput {
                execution_mode: CodeExecutionMode::Local,
                execution_root: root.to_string_lossy().into_owned(),
                worktree_id: None,
            })
            .expect("local roots are shareable");
    }

    #[test]
    fn corrupt_store_fails_closed_and_is_preserved() {
        let (directory, store) = store();
        let corrupt = b"{ not valid JSON";
        fs::write(store.store_path(), corrupt).expect("corrupt fixture");
        let local_root = directory.path().join("local-checkout");
        fs::create_dir(&local_root).expect("local root");
        let local_root = local_root.canonicalize().expect("canonical local root");

        assert!(store.load().is_err());
        let attempted = CodeThreadBinding {
            community_id: "community".to_string(),
            project_dtag: "project".to_string(),
            repository_identity: repository_identity('a'),
            codex_thread_id: "thread".to_string(),
            execution_mode: CodeExecutionMode::Local,
            execution_root: local_root.to_string_lossy().into_owned(),
            base_ref: "0123456789abcdef0123456789abcdef01234567".to_string(),
            worktree_id: None,
        };
        assert!(store.upsert(attempted).is_err());
        assert_eq!(
            fs::read(store.store_path()).expect("preserved bytes"),
            corrupt
        );
    }

    #[test]
    fn unversioned_and_unsupported_schemas_fail_closed() {
        let (_directory, store) = store();
        for fixture in [
            r#"{"bindings":[]}"#,
            r#"{"version":0,"bindings":[]}"#,
            r#"{"version":99,"bindings":[]}"#,
        ] {
            fs::write(store.store_path(), fixture).expect("schema fixture");
            assert!(store.load().is_err(), "fixture should fail: {fixture}");
            assert_eq!(
                fs::read_to_string(store.store_path()).expect("preserved schema"),
                fixture
            );
        }
    }

    #[test]
    fn oversized_store_is_rejected_before_json_parsing() {
        let (_directory, store) = store();
        let oversized = vec![b' '; MAX_BINDING_STORE_BYTES as usize + 1];
        fs::write(store.store_path(), oversized).expect("oversized fixture");

        let error = store.load().expect_err("oversized store must fail");
        assert!(error.contains("byte limit"), "unexpected error: {error}");
    }

    #[test]
    fn worktree_ids_require_canonical_hyphenated_uuids() {
        let (directory, store) = store();
        let simple_uuid = "11111111111141118111111111111111";
        let record = binding(
            &directory.path().join("managed-root"),
            scope("community", "project", 'a'),
            "thread-simple-uuid",
            CodeExecutionMode::Worktree,
            Some(simple_uuid),
        );

        assert!(store.upsert(record).is_err());
        assert!(store
            .ensure_execution_available(&CodeExecutionAvailabilityInput {
                execution_mode: CodeExecutionMode::Worktree,
                execution_root: directory
                    .path()
                    .join("managed-root")
                    .to_string_lossy()
                    .into_owned(),
                worktree_id: Some(simple_uuid.to_string()),
            })
            .is_err());
    }

    #[test]
    fn local_execution_root_must_exist_and_be_a_directory() {
        let (directory, store) = store();
        let missing = binding(
            &directory.path().join("missing-local-root"),
            scope("community", "project", 'a'),
            "thread-missing",
            CodeExecutionMode::Local,
            None,
        );
        assert!(store.upsert(missing).is_err());

        let file_root = directory.path().join("local-file");
        fs::write(&file_root, b"not a directory").expect("file root");
        let file_binding = binding(
            &file_root,
            scope("community", "project", 'a'),
            "thread-file",
            CodeExecutionMode::Local,
            None,
        );
        assert!(store.upsert(file_binding).is_err());
    }

    #[test]
    fn moved_local_root_does_not_block_a_healthy_binding_in_another_scope() {
        let (directory, store) = store();
        let stale_scope = scope("community-stale", "project-stale", 'a');
        let stale_root = directory.path().join("stale-local-root");
        fs::create_dir(&stale_root).expect("stale local root");
        let stale_root = stale_root.canonicalize().expect("canonical stale root");
        let stale_binding = binding(
            &stale_root,
            stale_scope.clone(),
            "thread-stale",
            CodeExecutionMode::Local,
            None,
        );
        store
            .upsert(stale_binding.clone())
            .expect("persist stale binding before root moves");

        let moved_root = directory.path().join("moved-local-root");
        fs::rename(&stale_root, &moved_root).expect("move stale local root");
        assert!(!stale_root.exists());

        let healthy_scope = scope("community-healthy", "project-healthy", 'b');
        let healthy_root = directory.path().join("healthy-local-root");
        fs::create_dir(&healthy_root).expect("healthy local root");
        let healthy_root = healthy_root.canonicalize().expect("canonical healthy root");
        let healthy_binding = binding(
            &healthy_root,
            healthy_scope.clone(),
            "thread-healthy",
            CodeExecutionMode::Local,
            None,
        );
        store
            .upsert(healthy_binding.clone())
            .expect("persist healthy binding despite stale root");

        let reloaded = CodeThreadBindingStore::for_app_data(directory.path())
            .expect("reopen mixed-availability store");
        assert_eq!(
            reloaded.list(&healthy_scope).expect("list healthy scope"),
            vec![healthy_binding]
        );
        assert_eq!(
            reloaded.list(&stale_scope).expect("list stale scope"),
            vec![stale_binding.clone()]
        );
        assert!(reloaded
            .load()
            .expect("load mixed-availability index")
            .bindings
            .contains(&stale_binding));
    }

    #[test]
    fn missing_managed_execution_root_remains_loadable() {
        let (directory, store) = store();
        let root = directory.path().join("pruned-managed-root");
        fs::create_dir(&root).expect("managed root");
        let root = root.canonicalize().expect("canonical managed root");
        let record = binding(
            &root,
            scope("community", "project", 'a'),
            "thread-pruned",
            CodeExecutionMode::Worktree,
            Some("11111111-1111-4111-8111-111111111111"),
        );

        store
            .upsert(record.clone())
            .expect("persist managed worktree");
        fs::remove_dir(&root).expect("simulate externally pruned worktree");
        assert_eq!(
            store.load().expect("recoverable binding").bindings,
            vec![record]
        );
    }

    #[test]
    fn non_not_found_execution_root_errors_are_not_treated_as_missing() {
        let (directory, store) = store();
        let blocking_file = directory.path().join("blocking-file");
        fs::write(&blocking_file, b"not a parent directory").expect("blocking file");
        let record = binding(
            &blocking_file.join("managed-root"),
            scope("community", "project", 'a'),
            "thread-invalid-parent",
            CodeExecutionMode::Worktree,
            Some("11111111-1111-4111-8111-111111111111"),
        );

        let error = store.upsert(record).expect_err("invalid parent must fail");
        assert!(
            error.contains("failed to inspect SchoolX Code execution root"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn mode_and_path_invariants_are_validated() {
        let (directory, store) = store();
        let local_root = directory.path().join("local");
        fs::create_dir(&local_root).expect("local root");
        let local_root = local_root.canonicalize().expect("canonical local root");
        let local_with_id = binding(
            &local_root,
            scope("community", "project", 'a'),
            "thread-local",
            CodeExecutionMode::Local,
            Some("11111111-1111-4111-8111-111111111111"),
        );
        let worktree_without_id = binding(
            &directory.path().join("worktree"),
            scope("community", "project", 'a'),
            "thread-worktree",
            CodeExecutionMode::Worktree,
            None,
        );
        let relative = binding(
            Path::new("relative"),
            scope("community", "project", 'a'),
            "thread-relative",
            CodeExecutionMode::Local,
            None,
        );

        assert!(store.upsert(local_with_id).is_err());
        assert!(store.upsert(worktree_without_id).is_err());
        assert!(store.upsert(relative).is_err());
        assert!(store.load().expect("still empty").bindings.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn execution_root_symlink_is_rejected() {
        use std::os::unix::fs::symlink;

        let (directory, store) = store();
        let real_root = directory.path().join("real-local-root");
        let linked_root = directory.path().join("linked-local-root");
        fs::create_dir(&real_root).expect("real local root");
        symlink(&real_root, &linked_root).expect("linked local root");
        let record = binding(
            &linked_root,
            scope("community", "project", 'a'),
            "thread-linked",
            CodeExecutionMode::Local,
            None,
        );

        assert!(store.upsert(record).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn binding_file_symlink_is_rejected_without_touching_target() {
        use std::os::unix::fs::symlink;

        let (directory, store) = store();
        let external = directory.path().join("external.json");
        fs::write(&external, b"outside").expect("external fixture");
        symlink(&external, store.store_path()).expect("binding symlink");

        assert!(store.load().is_err());
        assert_eq!(fs::read(&external).expect("external target"), b"outside");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_code_parent_is_rejected() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temp app data");
        let external = directory.path().join("external-code");
        fs::create_dir(&external).expect("external code dir");
        symlink(&external, directory.path().join(CODE_STORE_DIRECTORY))
            .expect("code directory symlink");

        assert!(CodeThreadBindingStore::for_app_data(directory.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn code_parent_replaced_with_symlink_after_open_is_rejected() {
        use std::os::unix::fs::symlink;

        let (directory, store) = store();
        let external = directory.path().join("external-code");
        fs::create_dir(&external).expect("external code dir");
        fs::remove_dir(directory.path().join(CODE_STORE_DIRECTORY)).expect("remove code dir");
        symlink(&external, directory.path().join(CODE_STORE_DIRECTORY))
            .expect("replacement code symlink");

        assert!(store.load().is_err());
        assert!(fs::read_dir(&external)
            .expect("external directory")
            .next()
            .is_none());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_app_data_root_is_rejected() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().expect("temp root");
        let real = directory.path().join("real-app-data");
        let link = directory.path().join("linked-app-data");
        fs::create_dir(&real).expect("real app data");
        symlink(&real, &link).expect("app-data symlink");

        assert!(CodeThreadBindingStore::for_app_data(&link).is_err());
    }
}
