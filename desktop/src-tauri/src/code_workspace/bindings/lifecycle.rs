//! Versioned binding lifecycle persistence and exact transition primitives.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    removal, validate_identifier, CodeExecutionMode, CodeThreadBinding, CodeThreadBindingIndex,
    CodeThreadBindingLookupInput, CodeThreadBindingScope, CodeThreadBindingStore,
    CodeThreadPreparation, CodeThreadPreparationOperation, CodeThreadPreparationState,
    CODE_THREAD_BINDING_SCHEMA_VERSION,
};

/// Frontend-safe projection of a binding lifecycle.
///
/// Operation identifiers and reconciliation targets deliberately remain native-only.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CodeThreadLifecycleStatus {
    Active,
    Archiving,
    Archived,
    Unarchiving,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum CodeThreadLifecycleTarget {
    Active,
    Archived,
}

impl CodeThreadLifecycleTarget {
    fn status(self) -> CodeThreadLifecycleStatus {
        match self {
            Self::Active => CodeThreadLifecycleStatus::Active,
            Self::Archived => CodeThreadLifecycleStatus::Archived,
        }
    }
}

/// Native-only delivery journal stored beside, not inside, the public binding.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
enum CodeThreadLifecycle {
    Active {},
    Archiving {
        operation_id: String,
    },
    Archived {},
    Unarchiving {
        operation_id: String,
    },
    Unknown {
        operation_id: String,
        target: CodeThreadLifecycleTarget,
    },
}

impl CodeThreadLifecycle {
    fn status(&self) -> CodeThreadLifecycleStatus {
        match self {
            Self::Active {} => CodeThreadLifecycleStatus::Active,
            Self::Archiving { .. } => CodeThreadLifecycleStatus::Archiving,
            Self::Archived {} => CodeThreadLifecycleStatus::Archived,
            Self::Unarchiving { .. } => CodeThreadLifecycleStatus::Unarchiving,
            Self::Unknown { .. } => CodeThreadLifecycleStatus::Unknown,
        }
    }

    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Active {} | Self::Archived {} => Ok(()),
            Self::Archiving { operation_id }
            | Self::Unarchiving { operation_id }
            | Self::Unknown { operation_id, .. } => validate_operation_id(operation_id),
        }
    }
}

/// Exact native lifecycle sibling for one public eight-field binding.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CodeThreadLifecycleRecord {
    community_id: String,
    project_dtag: String,
    repository_identity: String,
    codex_thread_id: String,
    lifecycle: CodeThreadLifecycle,
}

impl CodeThreadLifecycleRecord {
    fn active(binding: &CodeThreadBinding) -> Self {
        Self {
            community_id: binding.community_id.clone(),
            project_dtag: binding.project_dtag.clone(),
            repository_identity: binding.repository_identity.clone(),
            codex_thread_id: binding.codex_thread_id.clone(),
            lifecycle: CodeThreadLifecycle::Active {},
        }
    }

    fn lookup(&self) -> CodeThreadBindingLookupInput {
        CodeThreadBindingLookupInput {
            scope: CodeThreadBindingScope {
                community_id: self.community_id.clone(),
                project_dtag: self.project_dtag.clone(),
                repository_identity: self.repository_identity.clone(),
            },
            codex_thread_id: self.codex_thread_id.clone(),
        }
    }

    fn validate(&self) -> Result<(), String> {
        self.lookup().validate()?;
        self.lifecycle.validate()
    }
}

/// Native snapshot containing only the public binding and safe lifecycle status.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodeThreadBindingLifecycle {
    pub(crate) binding: CodeThreadBinding,
    pub(crate) status: CodeThreadLifecycleStatus,
}

/// Exact compare-and-swap token for one durable archive or unarchive attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodeThreadLifecycleClaim {
    lookup: CodeThreadBindingLookupInput,
    operation_id: String,
    previous: CodeThreadLifecycleTarget,
    target: CodeThreadLifecycleTarget,
}

impl CodeThreadLifecycleClaim {
    fn validate(&self) -> Result<(), String> {
        self.lookup.validate()?;
        validate_operation_id(&self.operation_id)?;
        if self.previous == self.target {
            return Err("SchoolX Code lifecycle claim cannot target its current state".to_string());
        }
        Ok(())
    }

    fn transitional_state(&self) -> Result<CodeThreadLifecycle, String> {
        match (self.previous, self.target) {
            (CodeThreadLifecycleTarget::Active, CodeThreadLifecycleTarget::Archived) => {
                Ok(CodeThreadLifecycle::Archiving {
                    operation_id: self.operation_id.clone(),
                })
            }
            (CodeThreadLifecycleTarget::Archived, CodeThreadLifecycleTarget::Active) => {
                Ok(CodeThreadLifecycle::Unarchiving {
                    operation_id: self.operation_id.clone(),
                })
            }
            _ => Err("SchoolX Code lifecycle claim has an invalid stable transition".to_string()),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CodeThreadBindingIndexV1 {
    version: u32,
    bindings: Vec<CodeThreadBinding>,
    #[serde(default)]
    preparations: Vec<CodeThreadPreparationV2>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CodeThreadBindingIndexV2 {
    version: u32,
    bindings: Vec<CodeThreadBinding>,
    lifecycles: Vec<CodeThreadLifecycleRecord>,
    #[serde(default)]
    preparations: Vec<CodeThreadPreparationV2>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CodeThreadBindingIndexV3 {
    version: u32,
    bindings: Vec<CodeThreadBinding>,
    lifecycles: Vec<CodeThreadLifecycleRecord>,
    #[serde(default)]
    preparations: Vec<CodeThreadPreparationV3>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CodeThreadPreparationV2 {
    preparation_id: String,
    community_id: String,
    project_dtag: String,
    repository_identity: String,
    execution_mode: CodeExecutionMode,
    execution_root: String,
    base_ref: String,
    worktree_id: Option<String>,
    state: CodeThreadPreparationState,
    #[serde(default)]
    recovery_thread_baseline: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CodeThreadPreparationV3 {
    preparation_id: String,
    community_id: String,
    project_dtag: String,
    repository_identity: String,
    execution_mode: CodeExecutionMode,
    execution_root: String,
    base_ref: String,
    worktree_id: Option<String>,
    operation: CodeThreadPreparationOperation,
    source_thread_id: Option<String>,
    state: CodeThreadPreparationState,
    #[serde(default)]
    recovery_thread_baseline: Option<Vec<String>>,
}

impl From<CodeThreadPreparationV2> for CodeThreadPreparation {
    fn from(preparation: CodeThreadPreparationV2) -> Self {
        Self {
            preparation_id: preparation.preparation_id,
            community_id: preparation.community_id,
            project_dtag: preparation.project_dtag,
            repository_identity: preparation.repository_identity,
            execution_mode: preparation.execution_mode,
            execution_root: preparation.execution_root,
            base_ref: preparation.base_ref,
            worktree_id: preparation.worktree_id,
            operation: CodeThreadPreparationOperation::Start,
            source_thread_id: None,
            state: preparation.state,
            recovery_thread_baseline: preparation.recovery_thread_baseline,
            merge_target_ref: None,
        }
    }
}

impl From<CodeThreadPreparationV3> for CodeThreadPreparation {
    fn from(preparation: CodeThreadPreparationV3) -> Self {
        Self {
            preparation_id: preparation.preparation_id,
            community_id: preparation.community_id,
            project_dtag: preparation.project_dtag,
            repository_identity: preparation.repository_identity,
            execution_mode: preparation.execution_mode,
            execution_root: preparation.execution_root,
            base_ref: preparation.base_ref,
            worktree_id: preparation.worktree_id,
            operation: preparation.operation,
            source_thread_id: preparation.source_thread_id,
            state: preparation.state,
            recovery_thread_baseline: preparation.recovery_thread_baseline,
            merge_target_ref: None,
        }
    }
}

pub(super) fn decode_binding_index(value: Value) -> Result<CodeThreadBindingIndex, String> {
    let version = value
        .as_object()
        .and_then(|object| object.get("version"))
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            "SchoolX Code binding index is missing a valid schema version".to_string()
        })?;
    match version {
        1 => {
            let legacy: CodeThreadBindingIndexV1 = serde_json::from_value(value)
                .map_err(|error| format!("SchoolX Code binding index is invalid: {error}"))?;
            if legacy.version != 1 {
                return Err(format!(
                    "unsupported SchoolX Code binding schema version {}",
                    legacy.version
                ));
            }
            let lifecycles = legacy
                .bindings
                .iter()
                .map(CodeThreadLifecycleRecord::active)
                .collect();
            Ok(CodeThreadBindingIndex {
                version: CODE_THREAD_BINDING_SCHEMA_VERSION,
                bindings: legacy.bindings,
                lifecycles,
                preparations: legacy
                    .preparations
                    .into_iter()
                    .map(CodeThreadPreparation::from)
                    .collect(),
                merge_targets: Vec::new(),
                removals: Default::default(),
            })
        }
        2 => {
            let legacy: CodeThreadBindingIndexV2 = serde_json::from_value(value)
                .map_err(|error| format!("SchoolX Code binding index is invalid: {error}"))?;
            if legacy.version != 2 {
                return Err(format!(
                    "unsupported SchoolX Code binding schema version {}",
                    legacy.version
                ));
            }
            Ok(CodeThreadBindingIndex {
                version: CODE_THREAD_BINDING_SCHEMA_VERSION,
                bindings: legacy.bindings,
                lifecycles: legacy.lifecycles,
                preparations: legacy
                    .preparations
                    .into_iter()
                    .map(CodeThreadPreparation::from)
                    .collect(),
                merge_targets: Vec::new(),
                removals: Default::default(),
            })
        }
        3 => {
            let legacy: CodeThreadBindingIndexV3 = serde_json::from_value(value)
                .map_err(|error| format!("SchoolX Code binding index is invalid: {error}"))?;
            if legacy.version != 3 {
                return Err(format!(
                    "unsupported SchoolX Code binding schema version {}",
                    legacy.version
                ));
            }
            Ok(CodeThreadBindingIndex {
                version: CODE_THREAD_BINDING_SCHEMA_VERSION,
                bindings: legacy.bindings,
                lifecycles: legacy.lifecycles,
                preparations: legacy
                    .preparations
                    .into_iter()
                    .map(CodeThreadPreparation::from)
                    .collect(),
                merge_targets: Vec::new(),
                removals: Default::default(),
            })
        }
        version if version == u64::from(CODE_THREAD_BINDING_SCHEMA_VERSION) => {
            serde_json::from_value(value)
                .map_err(|error| format!("SchoolX Code binding index is invalid: {error}"))
        }
        version if version > u64::from(CODE_THREAD_BINDING_SCHEMA_VERSION) => Err(format!(
            "SchoolX Code binding schema version {version} is newer than this build supports"
        )),
        version => Err(format!(
            "unsupported SchoolX Code binding schema version {version}"
        )),
    }
}

pub(super) fn validate_lifecycle_join(
    bindings: &[CodeThreadBinding],
    records: &[CodeThreadLifecycleRecord],
) -> Result<(), String> {
    let binding_keys = bindings
        .iter()
        .map(|binding| CodeThreadBindingLookupInput {
            scope: binding.scope(),
            codex_thread_id: binding.codex_thread_id.clone(),
        })
        .collect::<HashSet<_>>();
    let mut record_keys = HashSet::with_capacity(records.len());
    for record in records {
        record.validate()?;
        let lookup = record.lookup();
        if !record_keys.insert(lookup.clone()) {
            return Err(format!(
                "SchoolX Code binding index contains a duplicate lifecycle record for {}",
                lookup.codex_thread_id
            ));
        }
        if !binding_keys.contains(&lookup) {
            return Err(format!(
                "SchoolX Code binding index contains an orphan lifecycle record for {}",
                lookup.codex_thread_id
            ));
        }
    }
    if record_keys != binding_keys {
        return Err("SchoolX Code binding index is missing a lifecycle record".to_string());
    }
    Ok(())
}

pub(super) fn is_stably_archived(
    records: &[CodeThreadLifecycleRecord],
    input: &CodeThreadBindingLookupInput,
) -> bool {
    records
        .iter()
        .find(|record| record.lookup() == *input)
        .is_some_and(|record| matches!(record.lifecycle, CodeThreadLifecycle::Archived {}))
}

impl CodeThreadBindingIndex {
    /// Test stable Active authority from an already strictly loaded snapshot.
    pub(crate) fn has_stably_active_lifecycle(&self, input: &CodeThreadBindingLookupInput) -> bool {
        self.lifecycles
            .iter()
            .find(|record| record.lookup() == *input)
            .is_some_and(|record| matches!(record.lifecycle, CodeThreadLifecycle::Active {}))
    }
}

pub(super) fn contains_lifecycle(
    records: &[CodeThreadLifecycleRecord],
    input: &CodeThreadBindingLookupInput,
) -> bool {
    records.iter().any(|record| record.lookup() == *input)
}

pub(super) fn remove_exact_archived_lifecycle(
    records: &mut Vec<CodeThreadLifecycleRecord>,
    input: &CodeThreadBindingLookupInput,
) -> Result<(), String> {
    let position = records
        .iter()
        .position(|record| record.lookup() == *input)
        .ok_or_else(|| {
            "SchoolX Code removal finalization is missing its exact lifecycle".to_string()
        })?;
    if !matches!(
        records[position].lifecycle,
        CodeThreadLifecycle::Archived {}
    ) {
        return Err(
            "SchoolX Code removal finalization requires a stable Archived lifecycle".to_string(),
        );
    }
    records.remove(position);
    Ok(())
}

pub(super) fn sort_lifecycle_records(records: &mut [CodeThreadLifecycleRecord]) {
    records.sort_by(|left, right| {
        left.community_id
            .cmp(&right.community_id)
            .then_with(|| left.project_dtag.cmp(&right.project_dtag))
            .then_with(|| left.repository_identity.cmp(&right.repository_identity))
            .then_with(|| left.codex_thread_id.cmp(&right.codex_thread_id))
    });
}

pub(super) fn insert_active_lifecycle(
    records: &mut Vec<CodeThreadLifecycleRecord>,
    binding: &CodeThreadBinding,
) -> Result<(), String> {
    let record = CodeThreadLifecycleRecord::active(binding);
    let lookup = record.lookup();
    if records.iter().any(|existing| existing.lookup() == lookup) {
        return Err(format!(
            "SchoolX Code binding index already contains lifecycle state for {}",
            binding.codex_thread_id
        ));
    }
    records.push(record);
    Ok(())
}

impl CodeThreadBindingStore {
    /// Load one exact-scope managed-worktree authority snapshot for a
    /// read-only inventory projection.
    ///
    /// Bindings, lifecycle records, and unfinished preparations come from one
    /// validated v4 index load. Local checkouts are deliberately excluded and
    /// native-only recovery baselines are scrubbed before the snapshot leaves
    /// the store boundary.
    pub(crate) fn list_managed_inventory_authority(
        &self,
        scope: &CodeThreadBindingScope,
    ) -> Result<(Vec<CodeThreadBindingLifecycle>, Vec<CodeThreadPreparation>), String> {
        scope.validate()?;
        let index = self.load()?;
        let bindings = index
            .bindings
            .iter()
            .filter(|binding| {
                binding.is_in_scope(scope) && binding.execution_mode == CodeExecutionMode::Worktree
            })
            .map(|binding| lifecycle_snapshot(&index, binding))
            .collect::<Result<Vec<_>, _>>()?;
        let preparations = index
            .preparations
            .iter()
            .filter(|preparation| {
                preparation.is_in_scope(scope)
                    && preparation.execution_mode == CodeExecutionMode::Worktree
            })
            .cloned()
            .map(|mut preparation| {
                preparation.recovery_thread_baseline = None;
                preparation.merge_target_ref = None;
                preparation
            })
            .collect();
        Ok((bindings, preparations))
    }

    /// List exact-scope bindings with only their frontend-safe lifecycle status.
    pub(crate) fn list_with_lifecycle(
        &self,
        scope: &CodeThreadBindingScope,
    ) -> Result<Vec<CodeThreadBindingLifecycle>, String> {
        scope.validate()?;
        let index = self.load()?;
        index
            .bindings
            .iter()
            .filter(|binding| binding.is_in_scope(scope))
            .map(|binding| lifecycle_snapshot(&index, binding))
            .collect()
    }

    /// Look up one exact binding and its safe lifecycle projection.
    pub(crate) fn lookup_with_lifecycle(
        &self,
        input: &CodeThreadBindingLookupInput,
    ) -> Result<Option<CodeThreadBindingLifecycle>, String> {
        input.validate()?;
        let index = self.load()?;
        index
            .bindings
            .iter()
            .find(|binding| binding_matches(binding, input))
            .map(|binding| lifecycle_snapshot(&index, binding))
            .transpose()
    }

    /// Return an exact binding only when it is durably stable and active.
    pub(crate) fn require_active_binding(
        &self,
        input: &CodeThreadBindingLookupInput,
    ) -> Result<CodeThreadBinding, String> {
        let snapshot = self.lookup_with_lifecycle(input)?.ok_or_else(|| {
            "Codex thread is not bound to the requested SchoolX community, project, and repository"
                .to_string()
        })?;
        if snapshot.status != CodeThreadLifecycleStatus::Active {
            return Err(format!(
                "SchoolX Code binding is not executable while its lifecycle is {:?}",
                snapshot.status
            ));
        }
        Ok(snapshot.binding)
    }

    /// Durably claim an active binding for archive before any app-server RPC.
    pub(crate) fn begin_archive(
        &self,
        input: &CodeThreadBindingLookupInput,
    ) -> Result<CodeThreadLifecycleClaim, String> {
        self.begin_transition(input, CodeThreadLifecycleTarget::Archived)
    }

    /// Durably claim an archived binding for unarchive before any app-server RPC.
    pub(crate) fn begin_unarchive(
        &self,
        input: &CodeThreadBindingLookupInput,
    ) -> Result<CodeThreadLifecycleClaim, String> {
        self.begin_transition(input, CodeThreadLifecycleTarget::Active)
    }

    fn begin_transition(
        &self,
        input: &CodeThreadBindingLookupInput,
        target: CodeThreadLifecycleTarget,
    ) -> Result<CodeThreadLifecycleClaim, String> {
        self.begin_transition_with_save(input, target, |index| self.save(index))
    }

    fn begin_transition_with_save(
        &self,
        input: &CodeThreadBindingLookupInput,
        target: CodeThreadLifecycleTarget,
        save: impl FnOnce(&CodeThreadBindingIndex) -> Result<(), String>,
    ) -> Result<CodeThreadLifecycleClaim, String> {
        input.validate()?;
        let mut index = self.load()?;
        if removal::reserves_thread_id(&index, &input.codex_thread_id) {
            return Err(
                "SchoolX Code lifecycle cannot transition while removal state owns the thread"
                    .to_string(),
            );
        }
        let record = exact_lifecycle_mut(&mut index, input)?;
        let previous = match record.lifecycle {
            CodeThreadLifecycle::Active {} => CodeThreadLifecycleTarget::Active,
            CodeThreadLifecycle::Archived {} => CodeThreadLifecycleTarget::Archived,
            _ => {
                return Err(
                    "SchoolX Code binding already has an unfinished lifecycle operation"
                        .to_string(),
                );
            }
        };
        if previous == target {
            return Err(format!(
                "SchoolX Code binding is already stably {:?}",
                target.status()
            ));
        }
        let claim = CodeThreadLifecycleClaim {
            lookup: input.clone(),
            operation_id: uuid::Uuid::new_v4().hyphenated().to_string(),
            previous,
            target,
        };
        claim.validate()?;
        record.lifecycle = claim.transitional_state()?;
        index.sort();
        index.validate()?;
        save(&index)?;
        Ok(claim)
    }

    /// Restore the previous stable state only for the exact definitely-unsent claim.
    pub(crate) fn rollback_lifecycle_after_unsent(
        &self,
        claim: &CodeThreadLifecycleClaim,
    ) -> Result<CodeThreadBindingLifecycle, String> {
        claim.validate()?;
        let mut index = self.load()?;
        let record = exact_claim_record_mut(&mut index, claim)?;
        record.lifecycle = stable_lifecycle(claim.previous);
        self.save(&index)?;
        snapshot_for_lookup(&index, &claim.lookup)
    }

    /// Preserve an uncertain delivery as native-only unknown state.
    pub(crate) fn mark_lifecycle_unknown(
        &self,
        claim: &CodeThreadLifecycleClaim,
    ) -> Result<CodeThreadBindingLifecycle, String> {
        claim.validate()?;
        let mut index = self.load()?;
        let already_unknown = exact_lifecycle(&index, &claim.lookup)?.lifecycle
            == CodeThreadLifecycle::Unknown {
                operation_id: claim.operation_id.clone(),
                target: claim.target,
            };
        if !already_unknown {
            let record = exact_claim_record_mut(&mut index, claim)?;
            record.lifecycle = CodeThreadLifecycle::Unknown {
                operation_id: claim.operation_id.clone(),
                target: claim.target,
            };
            self.save(&index)?;
        }
        snapshot_for_lookup(&index, &claim.lookup)
    }

    /// Commit the exact successful claim to its stable target.
    pub(crate) fn complete_lifecycle_transition(
        &self,
        claim: &CodeThreadLifecycleClaim,
    ) -> Result<CodeThreadBindingLifecycle, String> {
        claim.validate()?;
        let mut index = self.load()?;
        let record = exact_claim_record_mut(&mut index, claim)?;
        record.lifecycle = stable_lifecycle(claim.target);
        self.save(&index)?;
        snapshot_for_lookup(&index, &claim.lookup)
    }

    /// Reconcile one record against exhaustive active/archived membership evidence.
    pub(crate) fn reconcile_lifecycle_membership(
        &self,
        input: &CodeThreadBindingLookupInput,
        active: bool,
        archived: bool,
    ) -> Result<CodeThreadBindingLifecycle, String> {
        input.validate()?;
        let mut index = self.load()?;
        let record = exact_lifecycle_mut(&mut index, input)?;
        let authoritative = exact_membership(active, archived);
        let mut changed = false;
        record.lifecycle = match (&record.lifecycle, authoritative) {
            (CodeThreadLifecycle::Active {}, Some(CodeThreadLifecycleTarget::Active))
            | (CodeThreadLifecycle::Archived {}, Some(CodeThreadLifecycleTarget::Archived)) => {
                record.lifecycle.clone()
            }
            (CodeThreadLifecycle::Active {}, _) => {
                changed = true;
                reconciliation_unknown(CodeThreadLifecycleTarget::Active)
            }
            (CodeThreadLifecycle::Archived {}, _) => {
                changed = true;
                reconciliation_unknown(CodeThreadLifecycleTarget::Archived)
            }
            (CodeThreadLifecycle::Archiving { .. }, Some(target)) => {
                changed = true;
                stable_lifecycle(target)
            }
            (CodeThreadLifecycle::Unarchiving { .. }, Some(target)) => {
                changed = true;
                stable_lifecycle(target)
            }
            (CodeThreadLifecycle::Unknown { .. }, Some(target)) => {
                changed = true;
                stable_lifecycle(target)
            }
            (CodeThreadLifecycle::Archiving { operation_id }, None) => {
                changed = true;
                CodeThreadLifecycle::Unknown {
                    operation_id: operation_id.clone(),
                    target: CodeThreadLifecycleTarget::Archived,
                }
            }
            (CodeThreadLifecycle::Unarchiving { operation_id }, None) => {
                changed = true;
                CodeThreadLifecycle::Unknown {
                    operation_id: operation_id.clone(),
                    target: CodeThreadLifecycleTarget::Active,
                }
            }
            (CodeThreadLifecycle::Unknown { .. }, None) => record.lifecycle.clone(),
        };
        if changed {
            self.save(&index)?;
        }
        snapshot_for_lookup(&index, input)
    }

    /// Fail closed after graph/reconciliation evidence fails for a stable binding.
    pub(crate) fn mark_stable_lifecycle_unknown(
        &self,
        input: &CodeThreadBindingLookupInput,
    ) -> Result<CodeThreadBindingLifecycle, String> {
        input.validate()?;
        let mut index = self.load()?;
        let record = exact_lifecycle_mut(&mut index, input)?;
        let target = match record.lifecycle {
            CodeThreadLifecycle::Active {} => CodeThreadLifecycleTarget::Active,
            CodeThreadLifecycle::Archived {} => CodeThreadLifecycleTarget::Archived,
            _ => {
                return Err(
                    "SchoolX Code can only mark a stable lifecycle unknown from new evidence"
                        .to_string(),
                );
            }
        };
        record.lifecycle = reconciliation_unknown(target);
        self.save(&index)?;
        snapshot_for_lookup(&index, input)
    }
}

fn binding_matches(binding: &CodeThreadBinding, input: &CodeThreadBindingLookupInput) -> bool {
    binding.codex_thread_id == input.codex_thread_id && binding.is_in_scope(&input.scope)
}

fn lifecycle_snapshot(
    index: &CodeThreadBindingIndex,
    binding: &CodeThreadBinding,
) -> Result<CodeThreadBindingLifecycle, String> {
    let lookup = CodeThreadBindingLookupInput {
        scope: binding.scope(),
        codex_thread_id: binding.codex_thread_id.clone(),
    };
    let record = exact_lifecycle(index, &lookup)?;
    Ok(CodeThreadBindingLifecycle {
        binding: binding.clone(),
        status: record.lifecycle.status(),
    })
}

fn snapshot_for_lookup(
    index: &CodeThreadBindingIndex,
    input: &CodeThreadBindingLookupInput,
) -> Result<CodeThreadBindingLifecycle, String> {
    let binding = index
        .bindings
        .iter()
        .find(|binding| binding_matches(binding, input))
        .ok_or_else(|| "SchoolX Code lifecycle binding disappeared".to_string())?;
    lifecycle_snapshot(index, binding)
}

fn exact_lifecycle<'a>(
    index: &'a CodeThreadBindingIndex,
    input: &CodeThreadBindingLookupInput,
) -> Result<&'a CodeThreadLifecycleRecord, String> {
    index
        .lifecycles
        .iter()
        .find(|record| record.lookup() == *input)
        .ok_or_else(|| {
            "SchoolX Code lifecycle was not found in the requested exact binding scope".to_string()
        })
}

fn exact_lifecycle_mut<'a>(
    index: &'a mut CodeThreadBindingIndex,
    input: &CodeThreadBindingLookupInput,
) -> Result<&'a mut CodeThreadLifecycleRecord, String> {
    index
        .lifecycles
        .iter_mut()
        .find(|record| record.lookup() == *input)
        .ok_or_else(|| {
            "SchoolX Code lifecycle was not found in the requested exact binding scope".to_string()
        })
}

fn exact_claim_record_mut<'a>(
    index: &'a mut CodeThreadBindingIndex,
    claim: &CodeThreadLifecycleClaim,
) -> Result<&'a mut CodeThreadLifecycleRecord, String> {
    let record = exact_lifecycle_mut(index, &claim.lookup)?;
    let exact_transition = record.lifecycle == claim.transitional_state()?;
    if !exact_transition {
        return Err(
            "SchoolX Code lifecycle changed after claim; stale transition was refused".to_string(),
        );
    }
    Ok(record)
}

fn stable_lifecycle(target: CodeThreadLifecycleTarget) -> CodeThreadLifecycle {
    match target {
        CodeThreadLifecycleTarget::Active => CodeThreadLifecycle::Active {},
        CodeThreadLifecycleTarget::Archived => CodeThreadLifecycle::Archived {},
    }
}

fn reconciliation_unknown(target: CodeThreadLifecycleTarget) -> CodeThreadLifecycle {
    CodeThreadLifecycle::Unknown {
        operation_id: uuid::Uuid::new_v4().hyphenated().to_string(),
        target,
    }
}

fn exact_membership(active: bool, archived: bool) -> Option<CodeThreadLifecycleTarget> {
    match (active, archived) {
        (true, false) => Some(CodeThreadLifecycleTarget::Active),
        (false, true) => Some(CodeThreadLifecycleTarget::Archived),
        (true, true) | (false, false) => None,
    }
}

fn validate_operation_id(value: &str) -> Result<(), String> {
    validate_identifier("lifecycle operation", value)?;
    let parsed = uuid::Uuid::parse_str(value)
        .map_err(|error| format!("SchoolX Code lifecycle operation id is not a UUID: {error}"))?;
    if parsed.hyphenated().to_string() != value {
        return Err(
            "SchoolX Code lifecycle operation id must be a canonical lowercase hyphenated UUID"
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
