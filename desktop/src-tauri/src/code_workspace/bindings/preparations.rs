use super::*;

impl CodeThreadBindingStore {
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
}
