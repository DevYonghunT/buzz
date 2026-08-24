import type {
  CodeExecutionMode,
  CodePreparedWorktree,
  CodeWorktreeStatus,
} from "../api/types";

/** Every New task dialog starts isolated; Local is never remembered. */
export const DEFAULT_CODE_TASK_EXECUTION_MODE: CodeExecutionMode = "worktree";

/** Display-only checkout state produced by native Git inspection. */
export type CodeLocalCheckoutSnapshot = {
  branch: string | null;
  dirty: boolean;
};

/** Keep path, ref, and Git OID authority out of the creation UI model. */
export function localCheckoutSnapshotFromPreparation(
  prepared: CodePreparedWorktree,
): CodeLocalCheckoutSnapshot {
  return {
    branch: prepared.worktree.branch,
    dirty: prepared.worktree.dirty,
  };
}

/** Project only native display fields from an execution-root revalidation. */
export function localCheckoutSnapshotFromStatus(
  status: CodeWorktreeStatus,
): CodeLocalCheckoutSnapshot {
  return { branch: status.branch, dirty: status.dirty };
}

/** Require another confirmation when the native-visible checkout state moved. */
export function localCheckoutSnapshotChanged(
  reviewed: CodeLocalCheckoutSnapshot,
  current: CodeLocalCheckoutSnapshot,
): boolean {
  return reviewed.branch !== current.branch || reviewed.dirty !== current.dirty;
}
