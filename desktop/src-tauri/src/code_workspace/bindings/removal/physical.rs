//! Private pinned physical-removal engine.
//!
//! The public command reaches this module only through
//! [`RemovalActivityClearance`], after proving the exact archived thread idle
//! and PTY-free while holding the app-wide binding lock. Startup uses the
//! recovery-only path for already durable claims.

use std::path::Path;
use std::sync::MutexGuard;
use std::time::Instant;

use super::{CodeThreadBindingStore, CodeWorktreeRemovalRecord};
use crate::code_workspace::runtime::CodeThreadIdleAdmissionGuard;
use crate::code_workspace::{CodeRuntime, CodeTerminalManager, CodeThreadBindingLookupInput};

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix;
#[cfg(target_os = "macos")]
pub(in crate::code_workspace) use unix::prepare_macos_removal_git;

/// Opaque proof that runtime and PTY admission checks were performed for one
/// exact lookup. Fields are private so arbitrary crate callers cannot forge a
/// physical claim from caller-supplied proof, paths, refs, or object ids.
#[allow(dead_code)]
pub(super) struct RemovalActivityClearance<'binding, 'runtime> {
    lookup: CodeThreadBindingLookupInput,
    _binding_guard: MutexGuard<'binding, ()>,
    _runtime_guard: CodeThreadIdleAdmissionGuard<'runtime>,
}

/// Opaque physical absence capability. Only the pinned inspector can create
/// it, and store finalization consumes it exactly once.
pub(super) struct VerifiedRemovalAbsence {
    expected_removing: CodeWorktreeRemovalRecord,
}

impl VerifiedRemovalAbsence {
    fn new(expected_removing: CodeWorktreeRemovalRecord) -> Self {
        Self { expected_removing }
    }

    pub(super) fn into_removing_record(self) -> CodeWorktreeRemovalRecord {
        self.expected_removing
    }
}

/// Prove the exact thread quiescent and free of user PTYs. The caller must hold
/// the application binding-store mutex from this proof through claim/execute.
#[allow(dead_code)]
pub(super) fn prove_removal_activity_clearance<'binding, 'runtime>(
    runtime: &'runtime CodeRuntime,
    terminals: &CodeTerminalManager,
    binding_guard: MutexGuard<'binding, ()>,
    lookup: CodeThreadBindingLookupInput,
) -> Result<RemovalActivityClearance<'binding, 'runtime>, String> {
    lookup.validate()?;
    runtime.ensure_thread_idle(&lookup.codex_thread_id)?;
    terminals.ensure_owner_absent(&lookup.scope, &lookup.codex_thread_id)?;
    let runtime_guard = runtime.lock_thread_idle_admission(&lookup.codex_thread_id)?;
    terminals.ensure_owner_absent(&lookup.scope, &lookup.codex_thread_id)?;
    Ok(RemovalActivityClearance {
        lookup,
        _binding_guard: binding_guard,
        _runtime_guard: runtime_guard,
    })
}

/// Claim and execute one private removal after native activity admission.
#[allow(dead_code)]
pub(super) fn remove_archived_worktree_private(
    store: &CodeThreadBindingStore,
    clearance: RemovalActivityClearance<'_, '_>,
    nest_root: &Path,
    deadline: Instant,
) -> Result<CodeWorktreeRemovalRecord, String> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        unix::claim_or_resume(
            store,
            &clearance.lookup,
            nest_root,
            deadline,
            &mut unix::NoopFaultHook,
        )
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (store, clearance, nest_root, deadline);
        Err("SchoolX Code pinned worktree removal is unsupported on this platform".to_string())
    }
}

/// Recover every durable claim before lifecycle/start/fork reconciliation.
/// Removed tombstones are also visited only for exact proof-ref/sidecar cleanup.
pub(crate) fn recover_pending_worktree_removals(
    store: &CodeThreadBindingStore,
    nest_root: &Path,
) -> Result<(), String> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        unix::recover_all(store, nest_root, &mut unix::NoopFaultHook)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = nest_root;
        reject_unsupported_pending_worktree_removals(store)
    }
}

/// Fail closed without mutation when this build has no pinned physical engine.
/// Kept platform-neutral so supported-host tests exercise the exact fallback
/// predicate used by unsupported production builds.
#[cfg(any(test, not(any(target_os = "linux", target_os = "macos"))))]
pub(super) fn reject_unsupported_pending_worktree_removals(
    store: &CodeThreadBindingStore,
) -> Result<(), String> {
    if store.list_pending_worktree_removals()?.is_empty() {
        Ok(())
    } else {
        Err("SchoolX Code pinned worktree removal is unsupported on this platform".to_string())
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests;
