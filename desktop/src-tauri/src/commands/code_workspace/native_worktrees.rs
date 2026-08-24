use super::*;

pub(super) fn prepare_worktree_native(
    input: CodeWorktreePrepareCommandInput,
    app_data_dir: &Path,
    nest_root: &Path,
    binding_lock: &Mutex<()>,
) -> Result<CodePreparedWorktree, String> {
    let _guard = lock_bindings(binding_lock)?;
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    store.ensure_preparation_capacity()?;
    input.scope.validate()?;
    let scope = input.scope.clone();
    let repository = preflight_execution_root(&input.repository_root, &input.base_ref)?;
    if scope.repository_identity != repository.repository_identity {
        return Err(
            "SchoolX Code preparation scope does not match the selected repository".to_string(),
        );
    }
    let prepared = crate::code_workspace::prepare_execution_root_with_merge_target(
        input.into_native(),
        nest_root,
    )?;
    let worktree = prepared.worktree;
    let preparation_id = uuid::Uuid::new_v4().hyphenated().to_string();
    store
        .create_preparation_with_merge_target(
            preparation_id.clone(),
            scope.clone(),
            &worktree.descriptor,
            prepared.merge_target_ref,
        )
        .map_err(|error| {
            format!(
                "SchoolX Code prepared execution root {} but could not journal it: {error}. The execution root was preserved",
                worktree.descriptor.execution_root
            )
        })?;
    Ok(CodePreparedWorktree {
        preparation_id,
        scope,
        worktree,
    })
}

pub(super) fn remove_worktree_native(
    input: CodeWorktreeRemoveInput,
    app_data_dir: &Path,
    nest_root: &Path,
    binding_lock: &Mutex<()>,
    context: CodeWorktreeRemovalContext<'_>,
) -> Result<CodeWorktreeRemovalReceipt, String> {
    input.validate()?;
    let binding_guard = lock_bindings(binding_lock)?;
    let store = CodeThreadBindingStore::for_app_data(app_data_dir)?;
    crate::code_workspace::git_write::ensure_admission_clear(
        app_data_dir,
        &input.scope,
        &input.thread_id,
    )?;
    crate::code_workspace::remove_archived_worktree(
        &store,
        binding_guard,
        input,
        nest_root,
        context,
    )
}

#[cfg(test)]
pub(crate) fn remove_worktree_for_test(
    input: CodeWorktreeRemoveInput,
    app_data_dir: &Path,
    nest_root: &Path,
    binding_lock: &Mutex<()>,
    context: (
        &crate::code_workspace::CodeRuntime,
        &crate::code_workspace::CodeTerminalManager,
        &std::sync::atomic::AtomicBool,
        &std::sync::atomic::AtomicBool,
    ),
) -> Result<CodeWorktreeRemovalReceipt, String> {
    remove_worktree_native(
        input,
        app_data_dir,
        nest_root,
        binding_lock,
        CodeWorktreeRemovalContext {
            runtime: context.0,
            terminals: context.1,
            lifecycle_authority_ready: context.2,
            shutdown_started: context.3,
        },
    )
}
