//! Startup ordering for the independent safe-remove and Git-write journals.
//!
//! Both journals are strictly loaded before either recovery engine runs. The
//! resulting snapshots are cross-joined by the complete binding coordinate so
//! one thread cannot be owned by both recovery protocols at startup.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::journal::{GitJournalBindingKey, GitOperationJournal, GitOperationJournalStore};
use crate::code_workspace::bindings::removal::pending_worktree_removal_keys;
use crate::code_workspace::bindings::CodeThreadBindingIndex;
use crate::code_workspace::{
    recover_pending_worktree_removals, CodeThreadBinding, CodeThreadBindingLookupInput,
    CodeThreadBindingStore,
};

#[derive(Clone, Debug)]
struct GitStartupRecoveryTarget {
    key: GitJournalBindingKey,
    binding: CodeThreadBinding,
    record_id: String,
}

#[derive(Debug)]
struct StartupRecoveryPlan {
    pending_removals: HashSet<GitJournalBindingKey>,
    git_targets: Vec<GitStartupRecoveryTarget>,
}

/// Strictly preflight and recover all startup-only native journals.
///
/// The caller must hold the application binding-store lock and must invoke
/// this only while the Codex runtime is stopped. Safe-remove recovery always
/// completes before Git recovery begins.
pub(crate) fn recover_startup_journals(
    binding_store: &CodeThreadBindingStore,
    app_data_dir: &Path,
    nest_root: &Path,
) -> Result<(), String> {
    recover_startup_journals_with(
        binding_store,
        app_data_dir,
        || recover_pending_worktree_removals(binding_store, nest_root),
        |target| {
            super::transaction::recover_record(app_data_dir, &target.binding, &target.record_id)
        },
    )
}

fn recover_startup_journals_with(
    binding_store: &CodeThreadBindingStore,
    app_data_dir: &Path,
    recover_removals: impl FnOnce() -> Result<(), String>,
    recover_git: impl FnMut(&GitStartupRecoveryTarget) -> Result<(), String>,
) -> Result<(), String> {
    // Keep this load order explicit. Neither strict load creates, repairs, or
    // rewrites a journal, so every preflight failure remains zero-mutation.
    let binding_index = binding_store.load()?;
    let git_journal = GitOperationJournalStore::for_app_data(app_data_dir)?.load()?;
    let plan = prepare_recovery_plan(&binding_index, &git_journal)?;
    if plan.pending_removals.is_empty() && plan.git_targets.is_empty() {
        execute_recovery_plan(plan, recover_removals, recover_git)
    } else {
        super::with_git_authority(|| execute_recovery_plan(plan, recover_removals, recover_git))
    }
}

fn prepare_recovery_plan(
    binding_index: &CodeThreadBindingIndex,
    git_journal: &GitOperationJournal,
) -> Result<StartupRecoveryPlan, String> {
    let pending_removals: HashSet<GitJournalBindingKey> =
        pending_worktree_removal_keys(binding_index)
            .into_iter()
            .map(|lookup| GitJournalBindingKey {
                scope: lookup.scope,
                thread_id: lookup.codex_thread_id,
            })
            .collect();

    // Cross-join every unacknowledged blocker before resolving or invoking
    // either recovery engine. The record order is durable and deterministic,
    // so a malformed cross-store state also produces a stable first error.
    for record in git_journal
        .records
        .iter()
        .filter(|record| record.phase.is_blocking())
    {
        if pending_removals.contains(&record.key) {
            return Err(overlap_error(&record.key));
        }
    }

    let mut bindings = HashMap::with_capacity(binding_index.bindings.len());
    for binding in &binding_index.bindings {
        let key = binding_key(binding);
        if bindings.insert(key, binding).is_some() {
            return Err(
                "SchoolX Code binding index contains duplicate exact startup coordinates"
                    .to_string(),
            );
        }
    }

    let mut git_targets = Vec::new();
    for record in git_journal
        .records
        .iter()
        .filter(|record| record.phase.is_blocking())
    {
        let binding = require_active_recovery_binding(binding_index, &bindings, &record.key)?;
        git_targets.push(GitStartupRecoveryTarget {
            key: record.key.clone(),
            binding,
            record_id: record.record_id.clone(),
        });
    }

    Ok(StartupRecoveryPlan {
        pending_removals,
        git_targets,
    })
}

fn execute_recovery_plan(
    plan: StartupRecoveryPlan,
    recover_removals: impl FnOnce() -> Result<(), String>,
    mut recover_git: impl FnMut(&GitStartupRecoveryTarget) -> Result<(), String>,
) -> Result<(), String> {
    for target in &plan.git_targets {
        if plan.pending_removals.contains(&target.key) {
            return Err(overlap_error(&target.key));
        }
    }

    recover_removals()?;
    for target in &plan.git_targets {
        recover_git(target)?;
    }
    Ok(())
}

fn binding_key(binding: &CodeThreadBinding) -> GitJournalBindingKey {
    GitJournalBindingKey {
        scope: binding.scope(),
        thread_id: binding.codex_thread_id.clone(),
    }
}

fn require_active_recovery_binding(
    binding_index: &CodeThreadBindingIndex,
    bindings: &HashMap<GitJournalBindingKey, &CodeThreadBinding>,
    key: &GitJournalBindingKey,
) -> Result<CodeThreadBinding, String> {
    let binding = bindings.get(key).ok_or_else(|| {
        format!(
            "SchoolX Code Git startup recovery has no exact live binding for thread {}",
            key.thread_id
        )
    })?;
    let lookup = CodeThreadBindingLookupInput {
        scope: key.scope.clone(),
        codex_thread_id: key.thread_id.clone(),
    };
    if !binding_index.has_stably_active_lifecycle(&lookup) {
        return Err(format!(
            "SchoolX Code Git startup recovery requires a stable Active lifecycle for thread {}",
            key.thread_id
        ));
    }
    Ok((*binding).clone())
}

fn overlap_error(key: &GitJournalBindingKey) -> String {
    format!(
        "SchoolX Code startup recovery found overlapping safe-remove and Git ownership for thread {}",
        key.thread_id
    )
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::fs;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;
    use crate::code_workspace::git_write::journal::MAX_GIT_OPERATION_JOURNAL_BYTES;
    use crate::code_workspace::{CodeExecutionMode, CodeThreadBindingScope};

    fn scope(marker: char) -> CodeThreadBindingScope {
        CodeThreadBindingScope {
            community_id: format!("community-{marker}"),
            project_dtag: format!("project-{marker}"),
            repository_identity: marker.to_string().repeat(64),
        }
    }

    fn target(scope: CodeThreadBindingScope, thread_id: &str) -> GitStartupRecoveryTarget {
        let key = GitJournalBindingKey {
            scope: scope.clone(),
            thread_id: thread_id.to_string(),
        };
        GitStartupRecoveryTarget {
            key,
            binding: CodeThreadBinding {
                community_id: scope.community_id,
                project_dtag: scope.project_dtag,
                repository_identity: scope.repository_identity,
                codex_thread_id: thread_id.to_string(),
                execution_mode: CodeExecutionMode::Local,
                execution_root: "/tmp/schoolx-startup-test".to_string(),
                base_ref: "a".repeat(40),
                worktree_id: None,
            },
            record_id: "b".repeat(64),
        }
    }

    fn binding_map(
        index: &CodeThreadBindingIndex,
    ) -> HashMap<GitJournalBindingKey, &CodeThreadBinding> {
        index
            .bindings
            .iter()
            .map(|binding| (binding_key(binding), binding))
            .collect()
    }

    #[cfg(unix)]
    fn write_private_journal(path: &Path, bytes: &[u8]) -> Result<(), String> {
        fs::write(path, bytes).map_err(|error| error.to_string())?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())
    }

    #[cfg(unix)]
    fn assert_invalid_journal_is_preserved_without_recovery(
        bytes: Vec<u8>,
        expected_error: &str,
    ) -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let binding_store = CodeThreadBindingStore::for_app_data(directory.path())?;
        let journal_store = GitOperationJournalStore::for_app_data(directory.path())?;
        let journal_path = journal_store.path().to_path_buf();
        write_private_journal(&journal_path, &bytes)?;
        let binding_path = binding_store.store_path().to_path_buf();
        assert!(!binding_path.exists());
        let safe_remove_calls = Cell::new(0);
        let git_calls = Cell::new(0);

        let error = recover_startup_journals_with(
            &binding_store,
            directory.path(),
            || {
                safe_remove_calls.set(safe_remove_calls.get() + 1);
                Ok(())
            },
            |_| {
                git_calls.set(git_calls.get() + 1);
                Ok(())
            },
        )
        .expect_err("invalid Git journal must fail strict startup preflight");

        assert!(
            error.contains(expected_error),
            "unexpected strict-load error: {error}"
        );
        assert_eq!(safe_remove_calls.get(), 0);
        assert_eq!(git_calls.get(), 0);
        assert_eq!(
            fs::read(&journal_path).map_err(|read_error| read_error.to_string())?,
            bytes
        );
        assert!(
            !binding_path.exists(),
            "strict binding load created an absent binding journal"
        );
        Ok(())
    }

    fn assert_non_active_lifecycle_fails_before_recovery(unknown: bool) -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let root = directory.path().join("execution-root");
        fs::create_dir(&root).map_err(|error| error.to_string())?;
        let root = root.canonicalize().map_err(|error| error.to_string())?;
        let owner = scope(if unknown { 'b' } else { 'a' });
        let thread_id = if unknown {
            "thread-unknown"
        } else {
            "thread-archived"
        };
        let store = CodeThreadBindingStore::for_app_data(directory.path())?;
        store.upsert(CodeThreadBinding {
            community_id: owner.community_id.clone(),
            project_dtag: owner.project_dtag.clone(),
            repository_identity: owner.repository_identity.clone(),
            codex_thread_id: thread_id.to_string(),
            execution_mode: CodeExecutionMode::Local,
            execution_root: root.to_string_lossy().into_owned(),
            base_ref: "a".repeat(40),
            worktree_id: None,
        })?;
        let lookup = CodeThreadBindingLookupInput {
            scope: owner.clone(),
            codex_thread_id: thread_id.to_string(),
        };
        let active_index = store.load()?;
        let key = GitJournalBindingKey {
            scope: owner,
            thread_id: thread_id.to_string(),
        };
        require_active_recovery_binding(&active_index, &binding_map(&active_index), &key)?;

        let claim = store.begin_archive(&lookup)?;
        if unknown {
            store.mark_lifecycle_unknown(&claim)?;
        } else {
            store.complete_lifecycle_transition(&claim)?;
        }
        let non_active_index = store.load()?;
        let safe_remove_calls = Cell::new(0);
        let git_calls = Cell::new(0);
        let error = (|| {
            let binding = require_active_recovery_binding(
                &non_active_index,
                &binding_map(&non_active_index),
                &key,
            )?;
            execute_recovery_plan(
                StartupRecoveryPlan {
                    pending_removals: HashSet::new(),
                    git_targets: vec![GitStartupRecoveryTarget {
                        key,
                        binding,
                        record_id: "c".repeat(64),
                    }],
                },
                || {
                    safe_remove_calls.set(safe_remove_calls.get() + 1);
                    Ok(())
                },
                |_| {
                    git_calls.set(git_calls.get() + 1);
                    Ok(())
                },
            )
        })()
        .expect_err("non-Active Git recovery binding must fail preflight");

        assert!(error.contains("stable Active lifecycle"));
        assert_eq!(safe_remove_calls.get(), 0);
        assert_eq!(git_calls.get(), 0);
        Ok(())
    }

    #[test]
    fn overlapping_exact_keys_fail_before_either_recovery_mutates() {
        let target = target(scope('a'), "thread-overlap");
        let plan = StartupRecoveryPlan {
            pending_removals: HashSet::from([target.key.clone()]),
            git_targets: vec![target],
        };
        let safe_remove_calls = Cell::new(0);
        let git_calls = Cell::new(0);

        let error = execute_recovery_plan(
            plan,
            || {
                safe_remove_calls.set(safe_remove_calls.get() + 1);
                Ok(())
            },
            |_| {
                git_calls.set(git_calls.get() + 1);
                Ok(())
            },
        )
        .expect_err("overlapping journals must fail closed");

        assert!(error.contains("overlapping safe-remove and Git ownership"));
        assert_eq!(safe_remove_calls.get(), 0);
        assert_eq!(git_calls.get(), 0);
    }

    #[test]
    fn same_thread_in_different_scopes_is_disjoint_and_ordered() {
        let removal_key = GitJournalBindingKey {
            scope: scope('a'),
            thread_id: "shared-thread".to_string(),
        };
        let plan = StartupRecoveryPlan {
            pending_removals: HashSet::from([removal_key]),
            git_targets: vec![target(scope('b'), "shared-thread")],
        };
        let order = RefCell::new(Vec::new());

        execute_recovery_plan(
            plan,
            || {
                order.borrow_mut().push("safe-remove");
                Ok(())
            },
            |_| {
                order.borrow_mut().push("git");
                Ok(())
            },
        )
        .expect("different exact scopes must not conflict");
        order.borrow_mut().push("runtime");

        assert_eq!(&*order.borrow(), &["safe-remove", "git", "runtime"]);
    }

    #[test]
    fn safe_remove_failure_prevents_git_recovery() {
        let plan = StartupRecoveryPlan {
            pending_removals: HashSet::new(),
            git_targets: vec![target(scope('a'), "thread")],
        };
        let git_calls = Cell::new(0);

        let error = execute_recovery_plan(
            plan,
            || Err("safe removal failed".to_string()),
            |_| {
                git_calls.set(git_calls.get() + 1);
                Ok(())
            },
        )
        .expect_err("Git recovery must wait for safe-remove recovery");

        assert_eq!(error, "safe removal failed");
        assert_eq!(git_calls.get(), 0);
    }

    #[test]
    fn archived_and_unknown_git_bindings_fail_before_recovery_mutation() -> Result<(), String> {
        assert_non_active_lifecycle_fails_before_recovery(false)?;
        assert_non_active_lifecycle_fails_before_recovery(true)
    }

    #[test]
    #[cfg(unix)]
    fn malformed_git_journal_bytes_are_preserved_before_any_recovery() -> Result<(), String> {
        assert_invalid_journal_is_preserved_without_recovery(
            br#"{"version":1,"records":["#.to_vec(),
            "failed to decode Git operation journal",
        )
    }

    #[test]
    #[cfg(unix)]
    fn oversized_git_journal_bytes_are_preserved_before_any_recovery() -> Result<(), String> {
        assert_invalid_journal_is_preserved_without_recovery(
            vec![b'x'; MAX_GIT_OPERATION_JOURNAL_BYTES as usize + 1],
            "exceeds",
        )
    }

    #[test]
    fn missing_git_journal_is_empty_without_creating_store_bytes() -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let binding_store = CodeThreadBindingStore::for_app_data(directory.path())?;
        let journal_path = GitOperationJournalStore::for_app_data(directory.path())?
            .path()
            .to_path_buf();
        let binding_path = binding_store.store_path().to_path_buf();
        assert!(!journal_path.exists());
        assert!(!binding_path.exists());
        let safe_remove_calls = Cell::new(0);
        let git_calls = Cell::new(0);

        recover_startup_journals_with(
            &binding_store,
            directory.path(),
            || {
                safe_remove_calls.set(safe_remove_calls.get() + 1);
                Ok(())
            },
            |_| {
                git_calls.set(git_calls.get() + 1);
                Ok(())
            },
        )?;

        assert_eq!(safe_remove_calls.get(), 1);
        assert_eq!(git_calls.get(), 0);
        assert!(
            !journal_path.exists(),
            "strict missing-journal load created Git store bytes"
        );
        assert!(
            !binding_path.exists(),
            "strict missing-journal load created binding store bytes"
        );
        Ok(())
    }
}
