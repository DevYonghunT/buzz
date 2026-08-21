//! Private journal and pinned engine for managed-worktree removal.
//!
//! The persisted v4 journal remains the authority/CAS boundary. Physical Git
//! and filesystem mutation lives only in the sealed child engine; the public
//! command can provide only an exact scope/thread coordinate.

use std::collections::HashSet;
use std::path::{Component, Path};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::MutexGuard;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::{
    lifecycle, validate_commit_id, validate_direct_local_branch_ref, validate_execution_root,
    validate_identifier, validate_sha256, validate_worktree_id, CodeExecutionMode,
    CodeThreadBinding, CodeThreadBindingIndex, CodeThreadBindingLookupInput,
    CodeThreadBindingStore, CodeWorktreeMergeTarget, MAX_EXECUTION_ROOT_BYTES,
};
use crate::code_workspace::worktrees::CodeMergeProofReceipt;

mod physical;

#[cfg(target_os = "macos")]
pub(in crate::code_workspace) use physical::prepare_macos_removal_git;
pub(crate) use physical::recover_pending_worktree_removals;

const MAX_REMOVALS: usize = 4_096;
const PUBLIC_REMOVAL_DEADLINE: Duration = Duration::from_secs(120);

/// Exact public coordinate accepted by managed-worktree safe removal.
///
/// All filesystem paths, Git refs and object ids, merge proof, and the
/// removal id remain native-derived authority.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeWorktreeRemoveInput {
    /// Community, project, and repository scope of the archived binding.
    pub scope: super::CodeThreadBindingScope,
    /// Exact Codex thread identifier of the archived binding.
    pub thread_id: String,
}

impl CodeWorktreeRemoveInput {
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.lookup().map(|_| ())
    }

    fn lookup(&self) -> Result<CodeThreadBindingLookupInput, String> {
        let lookup = CodeThreadBindingLookupInput {
            scope: self.scope.clone(),
            codex_thread_id: self.thread_id.clone(),
        };
        lookup.validate()?;
        Ok(lookup)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
enum CodeWorktreeRemovalLifecycleAtClaim {
    Archived,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
/// Final disposition of the Codex transcript after execution removal.
pub enum CodeWorktreeTranscriptDisposition {
    /// The Codex transcript remains at its native transcript coordinate.
    Preserved,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
/// Final disposition of the managed execution substrate.
pub enum CodeWorktreeExecutionDisposition {
    /// The exact managed root and reciprocal Git-admin entry were removed.
    Removed,
}

/// Native-derived receipt returned after verified physical removal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CodeWorktreeRemovalReceipt {
    /// Canonical native-issued UUID shared by every retry.
    pub removal_id: String,
    /// Durable scope copied from the removed binding tombstone.
    pub scope: super::CodeThreadBindingScope,
    /// Exact Codex thread identifier retained for transcript access.
    pub thread_id: String,
    /// Native managed-worktree UUID that was removed.
    pub worktree_id: String,
    /// Exact worktree HEAD proven before physical mutation.
    pub head_commit: String,
    /// Persisted direct local branch that contained the worktree HEAD.
    pub merged_into_ref: String,
    /// Exact commit at the persisted branch during the stable proof.
    pub merged_into_commit: String,
    /// Literal transcript disposition; removal never mutates the transcript.
    pub transcript_disposition: CodeWorktreeTranscriptDisposition,
    /// Literal physical execution disposition.
    pub execution_disposition: CodeWorktreeExecutionDisposition,
}

/// Pathnames frozen by a removal claim for a later handle-relative engine.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CodeWorktreeRemovalCoordinates {
    pub(crate) managed_root_parent: String,
    pub(crate) managed_root: String,
    pub(crate) quarantine_name: String,
    pub(crate) git_admin_parent: String,
    pub(crate) git_admin_entry: String,
}

/// Immutable authority shared by every state of one removal journal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CodeWorktreeRemovalAuthority {
    pub(crate) removal_id: String,
    pub(crate) binding: CodeThreadBinding,
    thread_lifecycle_at_claim: CodeWorktreeRemovalLifecycleAtClaim,
    pub(crate) merge_proof: CodeMergeProofReceipt,
    pub(crate) physical_manifest_digest: String,
    pub(crate) physical: CodeWorktreeRemovalCoordinates,
    transcript_disposition: CodeWorktreeTranscriptDisposition,
    execution_disposition: CodeWorktreeExecutionDisposition,
}

/// Strict v4 removal record. The internally tagged representation keeps the
/// persisted object flat while making the immutable payload identical across
/// `claimed`, `removing`, and `removed` CAS transitions.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum CodeWorktreeRemovalRecord {
    Claimed(CodeWorktreeRemovalAuthority),
    Removing(CodeWorktreeRemovalAuthority),
    Removed(CodeWorktreeRemovalAuthority),
}

/// Raw v4 probe used before `serde_json::Value` can collapse duplicate removal
/// members. Other index fields remain owned by the existing version decoder.
#[derive(Deserialize)]
struct V4RemovalWireProbe {
    #[serde(rename = "removals")]
    _removals: Vec<CodeWorktreeRemovalRecord>,
}

pub(super) fn validate_v4_removal_wire(bytes: &[u8]) -> Result<(), String> {
    serde_json::from_slice::<V4RemovalWireProbe>(bytes)
        .map(|_| ())
        .map_err(|error| format!("SchoolX Code v4 removal wire is invalid: {error}"))
}

impl CodeWorktreeRemovalRecord {
    pub(crate) fn authority(&self) -> &CodeWorktreeRemovalAuthority {
        match self {
            Self::Claimed(authority) | Self::Removing(authority) | Self::Removed(authority) => {
                authority
            }
        }
    }

    fn lookup(&self) -> CodeThreadBindingLookupInput {
        CodeThreadBindingLookupInput {
            scope: self.authority().binding.scope(),
            codex_thread_id: self.authority().binding.codex_thread_id.clone(),
        }
    }

    fn is_pending(&self) -> bool {
        matches!(self, Self::Claimed(_) | Self::Removing(_))
    }

    fn is_removed(&self) -> bool {
        matches!(self, Self::Removed(_))
    }

    fn removal_receipt(&self) -> Result<CodeWorktreeRemovalReceipt, String> {
        if !self.is_removed() {
            return Err(
                "SchoolX Code removal receipt requires a finalized removed tombstone".to_string(),
            );
        }
        self.validate()?;
        let authority = self.authority();
        let worktree_id = authority.binding.worktree_id.clone().ok_or_else(|| {
            "SchoolX Code removed tombstone is missing its managed worktree id".to_string()
        })?;
        Ok(CodeWorktreeRemovalReceipt {
            removal_id: authority.removal_id.clone(),
            scope: authority.binding.scope(),
            thread_id: authority.binding.codex_thread_id.clone(),
            worktree_id,
            head_commit: authority.merge_proof.head_commit.clone(),
            merged_into_ref: authority.merge_proof.target_ref.clone(),
            merged_into_commit: authority.merge_proof.target_commit.clone(),
            transcript_disposition: authority.transcript_disposition,
            execution_disposition: authority.execution_disposition,
        })
    }

    fn validate(&self) -> Result<(), String> {
        self.authority().validate()
    }

    fn with_removing_state(&self) -> Self {
        Self::Removing(self.authority().clone())
    }

    fn with_removed_state(&self) -> Self {
        Self::Removed(self.authority().clone())
    }
}

/// Project pending removal ownership from an already strictly loaded index.
///
/// Startup cross-preflight uses this read-only projection so it can join the
/// removal and Git journals before either recovery engine is allowed to
/// mutate durable state.
pub(crate) fn pending_worktree_removal_keys(
    index: &CodeThreadBindingIndex,
) -> HashSet<CodeThreadBindingLookupInput> {
    index
        .removals
        .iter()
        .filter(|record| record.is_pending())
        .map(CodeWorktreeRemovalRecord::lookup)
        .collect()
}

/// App-owned admission state consulted under the binding-store lock.
pub(crate) struct CodeWorktreeRemovalContext<'a> {
    pub(crate) runtime: &'a crate::code_workspace::CodeRuntime,
    pub(crate) terminals: &'a crate::code_workspace::CodeTerminalManager,
    pub(crate) lifecycle_authority_ready: &'a AtomicBool,
    pub(crate) shutdown_started: &'a AtomicBool,
}

/// Execute or retry one exact public safe-removal request while retaining the
/// application binding lock and native activity clearance through completion.
pub(crate) fn remove_archived_worktree(
    store: &CodeThreadBindingStore,
    binding_guard: MutexGuard<'_, ()>,
    input: CodeWorktreeRemoveInput,
    nest_root: &Path,
    context: CodeWorktreeRemovalContext<'_>,
) -> Result<CodeWorktreeRemovalReceipt, String> {
    let lookup = input.lookup()?;
    if let Some(existing) = store.lookup_worktree_removal(&lookup)? {
        if existing.is_removed() {
            return existing.removal_receipt();
        }
    }
    let shutting_down = context.shutdown_started.load(Ordering::Acquire);
    if shutting_down {
        return Err("SchoolX Code worktree removal cannot start during app shutdown".to_string());
    }
    if !context.lifecycle_authority_ready.load(Ordering::Acquire) {
        return Err(
            "SchoolX Code lifecycle authority is not ready for worktree removal".to_string(),
        );
    }

    let clearance = physical::prove_removal_activity_clearance(
        context.runtime,
        context.terminals,
        binding_guard,
        lookup,
    )?;
    physical::remove_archived_worktree_private(
        store,
        clearance,
        nest_root,
        Instant::now() + PUBLIC_REMOVAL_DEADLINE,
    )?
    .removal_receipt()
}

impl CodeWorktreeRemovalAuthority {
    fn validate(&self) -> Result<(), String> {
        validate_removal_id(&self.removal_id)?;
        self.binding.validate()?;
        if self.binding.execution_mode != CodeExecutionMode::Worktree {
            return Err(
                "SchoolX Code removal authority requires an original managed-worktree binding"
                    .to_string(),
            );
        }
        let worktree_id = self.binding.worktree_id.as_deref().ok_or_else(|| {
            "SchoolX Code removal authority is missing its original worktree id".to_string()
        })?;
        validate_merge_proof(&self.merge_proof)?;
        if self.merge_proof.repository_identity != self.binding.repository_identity
            || self.merge_proof.worktree_id != worktree_id
        {
            return Err(
                "SchoolX Code removal proof does not match its original managed binding"
                    .to_string(),
            );
        }
        let object_id_length = self.binding.base_ref.len();
        if self.merge_proof.head_commit.len() != object_id_length
            || self.merge_proof.target_commit.len() != object_id_length
        {
            return Err(
                "SchoolX Code removal proof mixes Git object-id formats for one repository"
                    .to_string(),
            );
        }
        validate_sha256(
            "removal physical manifest digest",
            &self.physical_manifest_digest,
        )?;
        self.physical
            .validate(&self.removal_id, &self.binding, worktree_id)
    }
}

impl CodeWorktreeRemovalCoordinates {
    fn validate(
        &self,
        removal_id: &str,
        binding: &CodeThreadBinding,
        worktree_id: &str,
    ) -> Result<(), String> {
        validate_absolute_coordinate("managed-root parent", &self.managed_root_parent)?;
        validate_absolute_coordinate("managed root", &self.managed_root)?;
        validate_absolute_coordinate("Git-admin parent", &self.git_admin_parent)?;
        validate_path_component("Git-admin entry", &self.git_admin_entry)?;
        validate_path_component("quarantine name", &self.quarantine_name)?;

        if self.managed_root != binding.execution_root {
            return Err(
                "SchoolX Code removal coordinate does not match the original execution root"
                    .to_string(),
            );
        }
        let managed_root = Path::new(&self.managed_root);
        if managed_root.parent() != Some(Path::new(&self.managed_root_parent))
            || managed_root.file_name().and_then(|name| name.to_str()) != Some(worktree_id)
        {
            return Err(
                "SchoolX Code removal root is not the exact worktree-id child of its parent"
                    .to_string(),
            );
        }
        let expected_quarantine = format!(".schoolx-removing-{removal_id}");
        if self.quarantine_name != expected_quarantine {
            return Err(
                "SchoolX Code removal quarantine name is not derived from its removal id"
                    .to_string(),
            );
        }
        Ok(())
    }
}

/// Native proof and physical metadata needed to create the first durable
/// `claimed` record. The removal id and quarantine name are always generated
/// by the store boundary, never supplied by a webview caller.
#[derive(Clone, Debug, Eq, PartialEq)]
struct CodeWorktreeRemovalClaimInput {
    lookup: CodeThreadBindingLookupInput,
    merge_proof: CodeMergeProofReceipt,
    physical_manifest_digest: String,
    git_admin_parent: String,
    git_admin_entry: String,
}

impl CodeWorktreeRemovalClaimInput {
    fn validate(&self) -> Result<(), String> {
        self.lookup.validate()?;
        validate_merge_proof(&self.merge_proof)?;
        validate_sha256(
            "removal physical manifest digest",
            &self.physical_manifest_digest,
        )?;
        validate_absolute_coordinate("Git-admin parent", &self.git_admin_parent)?;
        validate_path_component("Git-admin entry", &self.git_admin_entry)
    }
}

pub(super) fn validate_removal_join(index: &CodeThreadBindingIndex) -> Result<(), String> {
    if index.removals.len() > MAX_REMOVALS {
        return Err(format!(
            "SchoolX Code binding index exceeds the {MAX_REMOVALS}-removal limit"
        ));
    }

    let mut removal_ids = HashSet::with_capacity(index.removals.len());
    let mut retry_keys = HashSet::with_capacity(index.removals.len());
    let mut thread_ids = HashSet::with_capacity(index.removals.len());
    let mut worktree_ids = HashSet::with_capacity(index.removals.len());
    let mut execution_roots = HashSet::with_capacity(index.removals.len());

    for record in &index.removals {
        record.validate()?;
        let authority = record.authority();
        let lookup = record.lookup();
        let worktree_id = authority.binding.worktree_id.as_deref().ok_or_else(|| {
            "SchoolX Code removal record is missing its reserved worktree id".to_string()
        })?;

        if !removal_ids.insert(authority.removal_id.as_str()) {
            return Err(format!(
                "SchoolX Code binding index contains duplicate removal id {}",
                authority.removal_id
            ));
        }
        if !retry_keys.insert(lookup.clone()) {
            return Err(format!(
                "SchoolX Code binding index contains duplicate removal state for {}",
                lookup.codex_thread_id
            ));
        }
        if !thread_ids.insert(authority.binding.codex_thread_id.as_str())
            || !worktree_ids.insert(worktree_id)
            || !execution_roots.insert(authority.binding.execution_root.as_str())
        {
            return Err(
                "SchoolX Code removal records reuse a reserved thread, worktree, or root"
                    .to_string(),
            );
        }

        let exact_live_binding = index
            .bindings
            .iter()
            .filter(|binding| *binding == &authority.binding)
            .count();
        for binding in &index.bindings {
            let collides = binding.codex_thread_id == authority.binding.codex_thread_id
                || binding.worktree_id.as_deref() == Some(worktree_id)
                || binding.execution_root == authority.binding.execution_root;
            let allowed_pending_join = record.is_pending() && binding == &authority.binding;
            if collides && !allowed_pending_join {
                return Err(format!(
                    "SchoolX Code removal identity is reused by live binding {}",
                    binding.codex_thread_id
                ));
            }
        }

        if record.is_pending() {
            if exact_live_binding != 1 {
                return Err(format!(
                    "SchoolX Code pending removal is not joined to its exact live binding {}",
                    lookup.codex_thread_id
                ));
            }
            if !lifecycle::is_stably_archived(&index.lifecycles, &lookup) {
                return Err(format!(
                    "SchoolX Code pending removal requires a stable Archived lifecycle for {}",
                    lookup.codex_thread_id
                ));
            }
            let merge_target = exact_merge_target(index, &lookup).ok_or_else(|| {
                format!(
                    "SchoolX Code pending removal is missing merge-target authority for {}",
                    lookup.codex_thread_id
                )
            })?;
            if merge_target.worktree_id != worktree_id
                || merge_target.target_ref != authority.merge_proof.target_ref
            {
                return Err(format!(
                    "SchoolX Code pending removal merge authority changed for {}",
                    lookup.codex_thread_id
                ));
            }
        } else {
            if exact_live_binding != 0
                || lifecycle::contains_lifecycle(&index.lifecycles, &lookup)
                || exact_merge_target(index, &lookup).is_some()
            {
                return Err(format!(
                    "SchoolX Code removed tombstone still has live authority for {}",
                    lookup.codex_thread_id
                ));
            }
        }

        for preparation in &index.preparations {
            let reuses_destination = preparation.execution_root == authority.binding.execution_root
                || preparation.worktree_id.as_deref() == Some(worktree_id);
            let reuses_source = preparation.source_thread_id.as_deref()
                == Some(authority.binding.codex_thread_id.as_str());
            let reuses_recovery =
                preparation
                    .recovery_thread_baseline
                    .as_ref()
                    .is_some_and(|thread_ids| {
                        thread_ids
                            .iter()
                            .any(|thread_id| thread_id == &authority.binding.codex_thread_id)
                    });
            if reuses_destination || reuses_source || reuses_recovery {
                return Err(format!(
                    "SchoolX Code preparation reuses removal identity {}",
                    authority.binding.codex_thread_id
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn sort_removal_records(records: &mut [CodeWorktreeRemovalRecord]) {
    records.sort_by(|left, right| {
        let left = &left.authority().binding;
        let right = &right.authority().binding;
        left.community_id
            .cmp(&right.community_id)
            .then_with(|| left.project_dtag.cmp(&right.project_dtag))
            .then_with(|| left.repository_identity.cmp(&right.repository_identity))
            .then_with(|| left.codex_thread_id.cmp(&right.codex_thread_id))
    });
}

pub(super) fn reserves_thread_id(index: &CodeThreadBindingIndex, thread_id: &str) -> bool {
    index
        .removals
        .iter()
        .any(|record| record.authority().binding.codex_thread_id == thread_id)
}

pub(super) fn reserved_thread_ids(index: &CodeThreadBindingIndex) -> HashSet<String> {
    index
        .removals
        .iter()
        .map(|record| record.authority().binding.codex_thread_id.clone())
        .collect()
}

pub(super) fn reserves_execution(
    index: &CodeThreadBindingIndex,
    worktree_id: Option<&str>,
    execution_root: &str,
) -> bool {
    index.removals.iter().any(|record| {
        let binding = &record.authority().binding;
        binding.execution_root == execution_root
            || worktree_id
                .is_some_and(|worktree_id| binding.worktree_id.as_deref() == Some(worktree_id))
    })
}

#[cfg_attr(not(test), allow(dead_code))]
impl CodeThreadBindingStore {
    /// Return an existing claim or tombstone by its native retry key.
    pub(crate) fn lookup_worktree_removal(
        &self,
        input: &CodeThreadBindingLookupInput,
    ) -> Result<Option<CodeWorktreeRemovalRecord>, String> {
        input.validate()?;
        Ok(self
            .load()?
            .removals
            .into_iter()
            .find(|record| record.lookup() == *input))
    }

    /// Return only pending claims for startup recovery. Removed tombstones are
    /// permanent reservations, not executable recovery work.
    pub(crate) fn list_pending_worktree_removals(
        &self,
    ) -> Result<Vec<CodeWorktreeRemovalRecord>, String> {
        Ok(self
            .load()?
            .removals
            .into_iter()
            .filter(CodeWorktreeRemovalRecord::is_pending)
            .collect())
    }

    /// Return permanent tombstones only for exact post-finalization proof-ref
    /// and manifest cleanup. Tombstones never become executable recovery work.
    fn list_removed_worktree_tombstones(&self) -> Result<Vec<CodeWorktreeRemovalRecord>, String> {
        Ok(self
            .load()?
            .removals
            .into_iter()
            .filter(CodeWorktreeRemovalRecord::is_removed)
            .collect())
    }

    /// Refuse archived-capable commands while the exact live binding is owned
    /// by a claimed/removing removal. Removed tombstones are already absent
    /// from live binding lookup and therefore cannot reach this gate.
    pub(crate) fn ensure_no_pending_worktree_removal(
        &self,
        input: &CodeThreadBindingLookupInput,
    ) -> Result<(), String> {
        input.validate()?;
        if self
            .load()?
            .removals
            .iter()
            .any(|record| record.is_pending() && record.lookup() == *input)
        {
            return Err(
                "SchoolX Code thread is owned by pending worktree removal recovery".to_string(),
            );
        }
        Ok(())
    }

    /// Get or durably create one exact native removal claim. If a prior call
    /// committed but its response was lost, the existing record is returned
    /// without accepting replacement proof or coordinates.
    fn get_or_claim_worktree_removal(
        &self,
        input: &CodeWorktreeRemovalClaimInput,
    ) -> Result<CodeWorktreeRemovalRecord, String> {
        let removal_id = uuid::Uuid::new_v4().hyphenated().to_string();
        self.get_or_claim_worktree_removal_with_save(input, removal_id, |index| self.save(index))
    }

    fn get_or_claim_worktree_removal_with_save(
        &self,
        input: &CodeWorktreeRemovalClaimInput,
        removal_id: String,
        save: impl FnOnce(&CodeThreadBindingIndex) -> Result<(), String>,
    ) -> Result<CodeWorktreeRemovalRecord, String> {
        input.lookup.validate()?;
        let mut index = self.load()?;
        if let Some(existing) = index
            .removals
            .iter()
            .find(|record| record.lookup() == input.lookup)
        {
            return Ok(existing.clone());
        }
        input.validate()?;
        validate_removal_id(&removal_id)?;
        if index.removals.len() >= MAX_REMOVALS {
            return Err(format!(
                "SchoolX Code binding index reached the {MAX_REMOVALS}-removal limit"
            ));
        }

        let binding = index
            .bindings
            .iter()
            .find(|binding| {
                binding.codex_thread_id == input.lookup.codex_thread_id
                    && binding.is_in_scope(&input.lookup.scope)
            })
            .cloned()
            .ok_or_else(|| {
                "SchoolX Code removal claim requires an exact live binding".to_string()
            })?;
        if binding.execution_mode != CodeExecutionMode::Worktree {
            return Err(
                "SchoolX Code removal claim requires a managed-worktree binding".to_string(),
            );
        }
        if !lifecycle::is_stably_archived(&index.lifecycles, &input.lookup) {
            return Err(
                "SchoolX Code removal claim requires a stable Archived lifecycle".to_string(),
            );
        }
        let worktree_id = binding.worktree_id.as_deref().ok_or_else(|| {
            "SchoolX Code removal claim is missing its managed worktree id".to_string()
        })?;
        let merge_target = exact_merge_target(&index, &input.lookup).ok_or_else(|| {
            "SchoolX Code removal claim requires persisted merge-target authority".to_string()
        })?;
        if input.merge_proof.repository_identity != binding.repository_identity
            || input.merge_proof.worktree_id != worktree_id
            || input.merge_proof.target_ref != merge_target.target_ref
        {
            return Err(
                "SchoolX Code removal claim proof does not match its exact store authority"
                    .to_string(),
            );
        }

        let managed_root = binding.execution_root.clone();
        let managed_root_parent = Path::new(&managed_root)
            .parent()
            .and_then(Path::to_str)
            .ok_or_else(|| {
                "SchoolX Code removal root has no Unicode parent coordinate".to_string()
            })?
            .to_string();
        let record = CodeWorktreeRemovalRecord::Claimed(CodeWorktreeRemovalAuthority {
            removal_id: removal_id.clone(),
            binding,
            thread_lifecycle_at_claim: CodeWorktreeRemovalLifecycleAtClaim::Archived,
            merge_proof: input.merge_proof.clone(),
            physical_manifest_digest: input.physical_manifest_digest.clone(),
            physical: CodeWorktreeRemovalCoordinates {
                managed_root_parent,
                managed_root,
                quarantine_name: format!(".schoolx-removing-{removal_id}"),
                git_admin_parent: input.git_admin_parent.clone(),
                git_admin_entry: input.git_admin_entry.clone(),
            },
            transcript_disposition: CodeWorktreeTranscriptDisposition::Preserved,
            execution_disposition: CodeWorktreeExecutionDisposition::Removed,
        });
        record.validate()?;
        index.removals.push(record.clone());
        index.sort();
        index.validate()?;
        save(&index)?;
        Ok(record)
    }

    /// Persist the sticky `removing` boundary before any Git or filesystem
    /// mutation. Replaying the exact claimed CAS after response loss returns
    /// the already-advanced record or tombstone.
    fn mark_worktree_removal_removing(
        &self,
        expected_claimed: &CodeWorktreeRemovalRecord,
    ) -> Result<CodeWorktreeRemovalRecord, String> {
        self.mark_worktree_removal_removing_with_save(expected_claimed, |index| self.save(index))
    }

    fn mark_worktree_removal_removing_with_save(
        &self,
        expected_claimed: &CodeWorktreeRemovalRecord,
        save: impl FnOnce(&CodeThreadBindingIndex) -> Result<(), String>,
    ) -> Result<CodeWorktreeRemovalRecord, String> {
        if !matches!(expected_claimed, CodeWorktreeRemovalRecord::Claimed(_)) {
            return Err(
                "SchoolX Code removing transition requires the exact claimed record".to_string(),
            );
        }
        expected_claimed.validate()?;
        let mut index = self.load()?;
        let position = exact_removal_position(&index, expected_claimed)?;
        let current = index.removals[position].clone();
        if current.authority() != expected_claimed.authority() {
            return Err(
                "SchoolX Code removal authority changed after claim; stale CAS was refused"
                    .to_string(),
            );
        }
        if matches!(
            current,
            CodeWorktreeRemovalRecord::Removing(_) | CodeWorktreeRemovalRecord::Removed(_)
        ) {
            return Ok(current);
        }
        if current != *expected_claimed {
            return Err("SchoolX Code claimed removal changed; stale CAS was refused".to_string());
        }
        let removing = current.with_removing_state();
        index.removals[position] = removing.clone();
        index.sort();
        index.validate()?;
        save(&index)?;
        Ok(removing)
    }

    /// Cancel only an exact `claimed` record when the future deletion engine
    /// has proved that no first mutation was started. `removing` and `removed`
    /// can never use this path.
    fn cancel_claimed_worktree_removal_definitely_not_started(
        &self,
        expected_claimed: &CodeWorktreeRemovalRecord,
    ) -> Result<(), String> {
        self.cancel_claimed_worktree_removal_with_save(expected_claimed, |index| self.save(index))
    }

    fn cancel_claimed_worktree_removal_with_save(
        &self,
        expected_claimed: &CodeWorktreeRemovalRecord,
        save: impl FnOnce(&CodeThreadBindingIndex) -> Result<(), String>,
    ) -> Result<(), String> {
        if !matches!(expected_claimed, CodeWorktreeRemovalRecord::Claimed(_)) {
            return Err(
                "SchoolX Code removal cancellation requires the exact claimed record".to_string(),
            );
        }
        expected_claimed.validate()?;
        let mut index = self.load()?;
        let Some(position) = index
            .removals
            .iter()
            .position(|record| record.lookup() == expected_claimed.lookup())
        else {
            // An exact cancellation may have committed before its response was
            // lost. With no replacement at the retry key, absence is the
            // idempotent definitely-not-started result.
            return Ok(());
        };
        if index.removals[position] != *expected_claimed {
            return Err(
                "SchoolX Code removal changed after claim; cancellation was refused".to_string(),
            );
        }
        index.removals.remove(position);
        index.sort();
        index.validate()?;
        save(&index)
    }

    /// Atomically retire the live binding/lifecycle/merge authority into a
    /// permanent tombstone. This pure-store primitive may only be called by a
    /// future pinned engine after it verifies original/quarantine/admin absence.
    fn finalize_worktree_removal_after_verified_absence(
        &self,
        verified_absence: physical::VerifiedRemovalAbsence,
    ) -> Result<CodeWorktreeRemovalRecord, String> {
        let expected_removing = verified_absence.into_removing_record();
        self.finalize_worktree_removal_with_save(&expected_removing, |index| self.save(index))
    }

    /// Pure-store fault seam retained for journal tests. Production physical
    /// finalization can only consume `VerifiedRemovalAbsence` above.
    #[cfg(test)]
    fn finalize_worktree_removal_after_test_verified_absence(
        &self,
        expected_removing: &CodeWorktreeRemovalRecord,
    ) -> Result<CodeWorktreeRemovalRecord, String> {
        self.finalize_worktree_removal_with_save(expected_removing, |index| self.save(index))
    }

    fn finalize_worktree_removal_with_save(
        &self,
        expected_removing: &CodeWorktreeRemovalRecord,
        save: impl FnOnce(&CodeThreadBindingIndex) -> Result<(), String>,
    ) -> Result<CodeWorktreeRemovalRecord, String> {
        if !matches!(expected_removing, CodeWorktreeRemovalRecord::Removing(_)) {
            return Err(
                "SchoolX Code removal finalization requires the exact removing record".to_string(),
            );
        }
        expected_removing.validate()?;
        let mut index = self.load()?;
        let removal_position = exact_removal_position(&index, expected_removing)?;
        let current = index.removals[removal_position].clone();
        if current.authority() != expected_removing.authority() {
            return Err(
                "SchoolX Code removal authority changed before finalization; stale CAS was refused"
                    .to_string(),
            );
        }
        if matches!(current, CodeWorktreeRemovalRecord::Removed(_)) {
            return Ok(current);
        }
        if current != *expected_removing {
            return Err(
                "SchoolX Code removing state changed before finalization; stale CAS was refused"
                    .to_string(),
            );
        }

        let authority = current.authority().clone();
        let lookup = current.lookup();
        let binding_position = index
            .bindings
            .iter()
            .position(|binding| binding == &authority.binding)
            .ok_or_else(|| {
                "SchoolX Code removal finalization lost its exact live binding".to_string()
            })?;
        let merge_target_position = index
            .merge_targets
            .iter()
            .position(|target| {
                target.lookup() == lookup
                    && target.worktree_id == authority.merge_proof.worktree_id
                    && target.target_ref == authority.merge_proof.target_ref
            })
            .ok_or_else(|| {
                "SchoolX Code removal finalization lost its exact merge authority".to_string()
            })?;
        lifecycle::remove_exact_archived_lifecycle(&mut index.lifecycles, &lookup)?;
        index.bindings.remove(binding_position);
        index.merge_targets.remove(merge_target_position);
        let removed = current.with_removed_state();
        index.removals[removal_position] = removed.clone();
        index.sort();
        index.validate()?;
        save(&index)?;
        Ok(removed)
    }
}

fn exact_removal_position(
    index: &CodeThreadBindingIndex,
    expected: &CodeWorktreeRemovalRecord,
) -> Result<usize, String> {
    let lookup = expected.lookup();
    index
        .removals
        .iter()
        .position(|record| record.lookup() == lookup)
        .ok_or_else(|| {
            "SchoolX Code removal was not found for the exact CAS coordinate".to_string()
        })
}

fn exact_merge_target<'a>(
    index: &'a CodeThreadBindingIndex,
    lookup: &CodeThreadBindingLookupInput,
) -> Option<&'a CodeWorktreeMergeTarget> {
    index
        .merge_targets
        .iter()
        .find(|authority| authority.lookup() == *lookup)
}

fn validate_merge_proof(proof: &CodeMergeProofReceipt) -> Result<(), String> {
    validate_sha256(
        "removal proof repository identity",
        &proof.repository_identity,
    )?;
    validate_worktree_id(&proof.worktree_id)?;
    validate_commit_id(&proof.head_commit)?;
    validate_direct_local_branch_ref(&proof.target_ref)?;
    validate_commit_id(&proof.target_commit)
}

fn validate_removal_id(value: &str) -> Result<(), String> {
    validate_identifier("removal id", value)?;
    let parsed = uuid::Uuid::parse_str(value)
        .map_err(|error| format!("SchoolX Code removal id is not a UUID: {error}"))?;
    if parsed.hyphenated().to_string() != value {
        return Err(
            "SchoolX Code removal id must be a canonical lowercase hyphenated UUID".to_string(),
        );
    }
    if parsed.get_version() != Some(uuid::Version::Random) {
        return Err("SchoolX Code removal id must be a native UUID v4".to_string());
    }
    Ok(())
}

fn validate_absolute_coordinate(label: &str, value: &str) -> Result<(), String> {
    validate_execution_root(value)
        .map_err(|error| format!("SchoolX Code removal {label} coordinate is invalid: {error}"))?;
    if value.len() > MAX_EXECUTION_ROOT_BYTES {
        return Err(format!(
            "SchoolX Code removal {label} exceeds the {MAX_EXECUTION_ROOT_BYTES}-byte limit"
        ));
    }
    Ok(())
}

fn validate_path_component(label: &str, value: &str) -> Result<(), String> {
    validate_identifier(&format!("removal {label}"), value)?;
    let mut components = Path::new(value).components();
    let single_normal =
        matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none();
    if !single_normal || value.contains('/') || value.contains('\\') || matches!(value, "." | "..")
    {
        return Err(format!(
            "SchoolX Code removal {label} must be one safe path component"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
