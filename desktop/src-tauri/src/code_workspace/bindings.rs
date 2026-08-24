//! Durable SchoolX Code thread-to-execution-root bindings.
//!
//! Codex owns the thread transcript. SchoolX persists only the narrow index
//! needed to recover the native execution boundary after an app restart. The
//! stored execution root is authoritative once a thread is bound; callers may
//! not substitute a new path while resuming the thread or starting a turn.

use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::worktrees::CodeWorktreeDescriptor;

mod lifecycle;
mod preparations;
pub(crate) mod removal;
mod store;

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
mod tests;
