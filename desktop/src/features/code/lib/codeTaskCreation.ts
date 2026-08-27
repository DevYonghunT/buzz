import type {
  CodeExecutionMode,
  CodePreparedWorktree,
  CodeWorktreeStatus,
} from "../api/types";

/** Managed worktrees currently rely on native pinned-directory support. */
export function supportsManagedCodeWorktrees(platform?: string): boolean {
  const detectedPlatform =
    platform ?? (typeof navigator === "undefined" ? "" : navigator.platform);
  return !detectedPlatform.toLowerCase().startsWith("win");
}

/** Choose the safest execution mode supported by the current desktop OS. */
export function defaultCodeTaskExecutionMode(
  platform?: string,
): CodeExecutionMode {
  return supportsManagedCodeWorktrees(platform) ? "worktree" : "local";
}

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
