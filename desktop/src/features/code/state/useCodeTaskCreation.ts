import * as React from "react";

import {
  codeWorkspaceApi,
  getCodeThreadStartError,
} from "../api/codeWorkspace";
import type {
  CodeBoundThreadOpenResult,
  CodeExecutionMode,
  CodeModelSelection,
  CodePreparedWorktree,
  CodeThreadBindingScope,
  CodeThreadPreparation,
  CodeWorktreeDescriptor,
} from "../api/types";
import {
  type CodeLocalCheckoutSnapshot,
  defaultCodeTaskExecutionMode,
  localCheckoutSnapshotChanged,
  localCheckoutSnapshotFromPreparation,
  localCheckoutSnapshotFromStatus,
} from "../lib/codeTaskCreation";

export type CodeTaskCreationPhase =
  | "idle"
  | "preparing"
  | "revalidating"
  | "starting"
  | "refreshing";

type TaskPreparationAuthority = {
  preparationId: string;
  descriptor: CodeWorktreeDescriptor;
};

function taskAuthorityFromPrepared(
  prepared: CodePreparedWorktree,
): TaskPreparationAuthority {
  return {
    preparationId: prepared.preparationId,
    descriptor: prepared.worktree.descriptor,
  };
}

function taskAuthorityFromPersistedLocal(
  preparation: CodeThreadPreparation,
): TaskPreparationAuthority {
  return {
    preparationId: preparation.preparationId,
    // These fields are a native-issued descriptor projection and are passed
    // unchanged only through the existing read-only status contract.
    descriptor: {
      executionMode: preparation.executionMode,
      repositoryIdentity: preparation.repositoryIdentity,
      executionRoot: preparation.executionRoot,
      baseRef: preparation.baseRef,
      worktreeId: preparation.worktreeId,
    },
  };
}

function errorMessage(error: unknown): string {
  const threadStartError = getCodeThreadStartError(error);
  if (threadStartError) return threadStartError.message;
  return error instanceof Error ? error.message : String(error);
}

export function useCodeTaskCreation({
  baseRef,
  enabled,
  modelSelection,
  onCreated,
  onRefreshLists,
  onThreadStartRejected,
  repositoryRoot,
  runtimeGeneration,
  scope,
}: {
  baseRef: string;
  enabled: boolean;
  modelSelection: CodeModelSelection | null;
  onCreated: (
    opened: CodeBoundThreadOpenResult,
    selection: CodeModelSelection | null,
  ) => void;
  onRefreshLists: () => Promise<void>;
  onThreadStartRejected: () => void;
  repositoryRoot: string;
  runtimeGeneration: number | null;
  scope: CodeThreadBindingScope;
}) {
  const [open, setOpen] = React.useState(false);
  const [executionMode, setExecutionMode] = React.useState<CodeExecutionMode>(
    defaultCodeTaskExecutionMode,
  );
  const [phase, setPhase] = React.useState<CodeTaskCreationPhase>("idle");
  const [prepared, setPrepared] =
    React.useState<TaskPreparationAuthority | null>(null);
  const [localSnapshot, setLocalSnapshot] =
    React.useState<CodeLocalCheckoutSnapshot | null>(null);
  const [localSnapshotChanged, setLocalSnapshotChanged] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [recoveryRequired, setRecoveryRequired] = React.useState(false);
  const mountedRef = React.useRef(true);
  const activeAttemptRef = React.useRef<object | null>(null);
  const dialogTriggerRef = React.useRef<HTMLElement | null>(null);
  const contextKey = JSON.stringify([
    runtimeGeneration,
    scope.communityId,
    scope.projectDtag,
    scope.repositoryIdentity,
    repositoryRoot,
    baseRef,
  ]);
  const contextKeyRef = React.useRef(contextKey);
  const previousContextKeyRef = React.useRef(contextKey);
  contextKeyRef.current = contextKey;
  const pending = phase !== "idle";

  const resetDraft = React.useCallback(() => {
    setExecutionMode(defaultCodeTaskExecutionMode());
    setPhase("idle");
    setPrepared(null);
    setLocalSnapshot(null);
    setLocalSnapshotChanged(false);
    setError(null);
    setRecoveryRequired(false);
  }, []);

  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      activeAttemptRef.current = null;
      dialogTriggerRef.current = null;
    };
  }, []);

  React.useEffect(() => {
    if (previousContextKeyRef.current === contextKey) return;
    previousContextKeyRef.current = contextKey;
    activeAttemptRef.current = null;
    dialogTriggerRef.current = null;
    setOpen(false);
    resetDraft();
  }, [contextKey, resetDraft]);

  const rememberDialogTrigger = React.useCallback(() => {
    dialogTriggerRef.current =
      typeof document !== "undefined" &&
      document.activeElement instanceof HTMLElement
        ? document.activeElement
        : null;
  }, []);

  const restoreDialogTriggerFocus = React.useCallback(() => {
    const trigger = dialogTriggerRef.current;
    dialogTriggerRef.current = null;
    if (trigger?.isConnected) trigger.focus();
  }, []);

  const openDialog = React.useCallback(() => {
    if (!enabled || activeAttemptRef.current !== null) return;
    rememberDialogTrigger();
    resetDraft();
    setOpen(true);
  }, [enabled, rememberDialogTrigger, resetDraft]);

  const openPersistedLocalPreparation = React.useCallback(
    (preparation: CodeThreadPreparation) => {
      if (
        !enabled ||
        activeAttemptRef.current !== null ||
        preparation.executionMode !== "local" ||
        preparation.operation !== "start" ||
        preparation.state !== "prepared" ||
        preparation.communityId !== scope.communityId ||
        preparation.projectDtag !== scope.projectDtag ||
        preparation.repositoryIdentity !== scope.repositoryIdentity
      ) {
        return;
      }
      rememberDialogTrigger();
      resetDraft();
      setExecutionMode("local");
      setPrepared(taskAuthorityFromPersistedLocal(preparation));
      setOpen(true);
    },
    [enabled, rememberDialogTrigger, resetDraft, scope],
  );

  const setDialogOpen = React.useCallback(
    (nextOpen: boolean) => {
      if (activeAttemptRef.current !== null) return;
      if (nextOpen) {
        rememberDialogTrigger();
        resetDraft();
        setOpen(true);
        return;
      }
      const refreshUnfinished = prepared !== null;
      setOpen(false);
      resetDraft();
      if (refreshUnfinished) {
        void onRefreshLists().catch(() => {
          // Closing never replaces the native action result with refresh copy.
        });
      }
    },
    [onRefreshLists, prepared, rememberDialogTrigger, resetDraft],
  );

  const selectExecutionMode = React.useCallback(
    (mode: CodeExecutionMode) => {
      if (activeAttemptRef.current !== null || prepared !== null) return;
      setExecutionMode(mode);
      setLocalSnapshot(null);
      setLocalSnapshotChanged(false);
      setError(null);
    },
    [prepared],
  );

  const submit = React.useCallback(async () => {
    if (
      !enabled ||
      !open ||
      runtimeGeneration === null ||
      recoveryRequired ||
      activeAttemptRef.current !== null
    ) {
      return;
    }
    const attempt = {};
    activeAttemptRef.current = attempt;
    const requestContextKey = contextKey;
    const requestMode = executionMode;
    const requestSelection = modelSelection;
    const ownsAttempt = () => activeAttemptRef.current === attempt;
    const isCurrent = () =>
      mountedRef.current &&
      ownsAttempt() &&
      contextKeyRef.current === requestContextKey;
    setError(null);
    let candidate = prepared;
    let threadStartAttempted = false;
    try {
      if (candidate === null) {
        setPhase("preparing");
        const preparedResult = await codeWorkspaceApi.prepareCodeWorktree({
          scope,
          repositoryRoot,
          baseRef,
          executionMode: requestMode,
        });
        if (!isCurrent()) return;
        if (preparedResult.worktree.descriptor.executionMode !== requestMode) {
          throw new Error(
            "SchoolX Code returned a different execution mode than requested.",
          );
        }
        candidate = taskAuthorityFromPrepared(preparedResult);
        setPrepared(candidate);
        if (requestMode === "local") {
          setLocalSnapshot(
            localCheckoutSnapshotFromPreparation(preparedResult),
          );
          setLocalSnapshotChanged(false);
          setPhase("idle");
          return;
        }
      }

      if (requestMode === "local") {
        const reviewed = localSnapshot;
        setPhase("revalidating");
        // Status receives only the exact native-issued descriptor. Thread start
        // still accepts scope + preparationId and resolves its root natively.
        const status = await codeWorkspaceApi.getCodeWorktreeStatus(
          candidate.descriptor,
        );
        if (!isCurrent()) return;
        const current = localCheckoutSnapshotFromStatus(status);
        setLocalSnapshot(current);
        if (reviewed === null) {
          setLocalSnapshotChanged(false);
          setPhase("idle");
          return;
        }
        if (localCheckoutSnapshotChanged(reviewed, current)) {
          setLocalSnapshotChanged(true);
          setPhase("idle");
          return;
        }
        setLocalSnapshotChanged(false);
      }

      setPhase("starting");
      threadStartAttempted = true;
      const opened = await codeWorkspaceApi.startCodeThread({
        scope,
        preparationId: candidate.preparationId,
        model: requestSelection?.model ?? null,
      });
      threadStartAttempted = false;
      if (!isCurrent()) return;
      onCreated(opened, requestSelection);
      setOpen(false);
      resetDraft();
    } catch (caught) {
      if (!isCurrent()) return;
      if (threadStartAttempted) {
        onThreadStartRejected();
        // The native preparation may now be `starting`; never issue another
        // start from stale UI state. The refreshed Unfinished row decides
        // whether the next action is start or binding recovery.
        setRecoveryRequired(true);
      }
      setError(errorMessage(caught));
      setPhase("refreshing");
      try {
        await onRefreshLists();
      } catch {
        // Preserve the native action error beside the recovery affordance.
      }
    } finally {
      if (ownsAttempt()) {
        activeAttemptRef.current = null;
        if (mountedRef.current && contextKeyRef.current === requestContextKey) {
          setPhase("idle");
        }
      }
    }
  }, [
    baseRef,
    contextKey,
    enabled,
    executionMode,
    localSnapshot,
    modelSelection,
    onCreated,
    onRefreshLists,
    onThreadStartRejected,
    open,
    prepared,
    recoveryRequired,
    repositoryRoot,
    resetDraft,
    runtimeGeneration,
    scope,
  ]);

  return {
    enabled: enabled && !recoveryRequired,
    error,
    executionMode,
    localSnapshot,
    localSnapshotChanged,
    open,
    openDialog,
    openPersistedLocalPreparation,
    pending,
    phase,
    preparationReady: prepared !== null,
    recoveryRequired,
    restoreDialogTriggerFocus,
    selectExecutionMode,
    setDialogOpen,
    submit,
  };
}
