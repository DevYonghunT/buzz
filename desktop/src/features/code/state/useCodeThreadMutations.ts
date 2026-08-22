import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { codeWorkspaceApi } from "../api/codeWorkspace";
import type {
  CodeBoundThreadOpenResult,
  CodeThreadBindingScope,
  CodeThreadPreparation,
  CodeThreadSummary,
  CodeThreadsPage,
} from "../api/types";
import {
  codeSessionQueryKeys,
  codeThreadPreparationsQueryOptions,
  codeThreadsQueryOptions,
} from "./codeSessionQueries";

type LifecycleMutation = "archive" | "unarchive";

function lifecycleRefreshError(
  mutationError: unknown,
  refreshError: unknown,
): Error {
  const mutationMessage =
    mutationError instanceof Error
      ? mutationError.message
      : String(mutationError);
  const refreshMessage =
    refreshError instanceof Error ? refreshError.message : String(refreshError);
  return new Error(
    `${mutationMessage} The task status could not be refreshed: ${refreshMessage}`,
  );
}

/**
 * Keeps lifecycle mutation uncertainty separate from durable native state.
 * Native remains authoritative; the local gate only prevents new interactions
 * until an exact list refetch confirms the persisted row.
 */
export function useCodeThreadMutations({
  mutationsReady,
  onThreadSnapshot,
  scope,
}: {
  mutationsReady: boolean;
  onThreadSnapshot: (threadId: string, thread: CodeThreadSummary) => void;
  scope: CodeThreadBindingScope;
}) {
  const queryClient = useQueryClient();
  const [pendingLifecycleThreadId, setPendingLifecycleThreadId] =
    React.useState<string | null>(null);
  const [pendingForkThreadId, setPendingForkThreadId] = React.useState<
    string | null
  >(null);
  const [unreconciledThreadIds, setUnreconciledThreadIds] = React.useState<
    ReadonlySet<string>
  >(() => new Set());
  const [unreconciledForkThreadIds, setUnreconciledForkThreadIds] =
    React.useState<ReadonlySet<string>>(() => new Set());
  const pendingMutationThreadIdRef = React.useRef<string | null>(null);
  const semanticForkBlockerThreadIdsRef = React.useRef(new Set<string>());

  const mutateLifecycle = React.useCallback(
    async (threadId: string, mutation: LifecycleMutation): Promise<void> => {
      if (!mutationsReady) {
        throw new Error("Code task actions are unavailable while syncing.");
      }
      if (pendingMutationThreadIdRef.current !== null) {
        throw new Error("Another Code task action is still pending.");
      }

      pendingMutationThreadIdRef.current = threadId;
      setPendingLifecycleThreadId(threadId);
      let mutationError: unknown = null;
      try {
        const input = { scope, threadId };
        if (mutation === "archive") {
          await codeWorkspaceApi.archiveCodeThread(input);
        } else {
          await codeWorkspaceApi.unarchiveCodeThread(input);
        }
      } catch (error) {
        mutationError = error;
      }
      void queryClient.invalidateQueries({
        queryKey: codeSessionQueryKeys.worktrees(scope),
      });

      let refreshError: unknown = null;
      try {
        const refreshed = await queryClient.fetchQuery({
          ...codeThreadsQueryOptions(scope),
          staleTime: 0,
        });
        if (
          !refreshed.data.some((row) => row.binding.codexThreadId === threadId)
        ) {
          throw new Error(
            "The mutated Code task was missing from its authoritative scope.",
          );
        }
        setUnreconciledThreadIds((current) => {
          if (!current.has(threadId)) return current;
          const next = new Set(current);
          next.delete(threadId);
          return next;
        });
      } catch (error) {
        refreshError = error;
        setUnreconciledThreadIds((current) => {
          const next = new Set(current);
          next.add(threadId);
          return next;
        });
      } finally {
        pendingMutationThreadIdRef.current = null;
        setPendingLifecycleThreadId(null);
      }

      if (refreshError !== null) {
        throw lifecycleRefreshError(
          mutationError ?? new Error("The lifecycle action completed."),
          refreshError,
        );
      }
      if (mutationError !== null) throw mutationError;
    },
    [mutationsReady, queryClient, scope],
  );

  const forkThread = React.useCallback(
    async (threadId: string): Promise<CodeBoundThreadOpenResult> => {
      if (!mutationsReady) {
        throw new Error("Code task actions are unavailable while syncing.");
      }
      if (pendingMutationThreadIdRef.current !== null) {
        throw new Error("Another Code task action is still pending.");
      }
      const source = queryClient
        .getQueryData<CodeThreadsPage>(codeSessionQueryKeys.threads(scope))
        ?.data.find((row) => row.binding.codexThreadId === threadId);
      if (
        source?.lifecycle !== "active" ||
        source.unavailable !== null ||
        source.binding.executionMode !== "worktree" ||
        source.binding.worktreeId === null
      ) {
        throw new Error(
          "Only an available active managed-worktree task can be forked.",
        );
      }

      pendingMutationThreadIdRef.current = threadId;
      setPendingForkThreadId(threadId);
      let opened: CodeBoundThreadOpenResult | null = null;
      let mutationError: unknown = null;
      let semanticResultError = false;
      try {
        const candidate = await codeWorkspaceApi.forkCodeThread({
          scope,
          threadId,
        });
        if (
          candidate.binding.executionRoot === source.binding.executionRoot ||
          candidate.binding.worktreeId === source.binding.worktreeId
        ) {
          semanticResultError = true;
          throw new TypeError(
            "Forked Code task must use a fresh managed destination.",
          );
        }
        opened = candidate;
      } catch (error) {
        mutationError = error;
      }
      if (opened !== null) {
        semanticForkBlockerThreadIdsRef.current.delete(threadId);
        pendingMutationThreadIdRef.current = null;
        setPendingForkThreadId(null);
        setUnreconciledForkThreadIds((current) => {
          if (!current.has(threadId)) return current;
          const next = new Set(current);
          next.delete(threadId);
          return next;
        });
        void queryClient.invalidateQueries({
          queryKey: codeSessionQueryKeys.preparations(scope),
        });
        void queryClient.invalidateQueries({
          queryKey: codeSessionQueryKeys.worktrees(scope),
          refetchType: "none",
        });
        return opened;
      }
      void queryClient.invalidateQueries({
        queryKey: codeSessionQueryKeys.worktrees(scope),
      });

      let refreshError: unknown = null;
      try {
        const [refreshed] = await Promise.all([
          queryClient.fetchQuery({
            ...codeThreadsQueryOptions(scope),
            staleTime: 0,
          }),
          queryClient.fetchQuery({
            ...codeThreadPreparationsQueryOptions(scope),
            staleTime: 0,
          }),
        ]);
        if (
          !refreshed.data.some((row) => row.binding.codexThreadId === threadId)
        ) {
          throw new Error(
            "The fork source was missing from its authoritative scope.",
          );
        }
        setUnreconciledForkThreadIds((current) => {
          if (!current.has(threadId) || semanticResultError) return current;
          const next = new Set(current);
          next.delete(threadId);
          return next;
        });
      } catch (error) {
        refreshError = error;
        setUnreconciledForkThreadIds((current) => {
          const next = new Set(current);
          next.add(threadId);
          return next;
        });
      } finally {
        pendingMutationThreadIdRef.current = null;
        setPendingForkThreadId(null);
      }

      if (semanticResultError) {
        semanticForkBlockerThreadIdsRef.current.add(threadId);
        setUnreconciledForkThreadIds((current) => {
          const next = new Set(current);
          next.add(threadId);
          return next;
        });
      }
      if (refreshError !== null) {
        throw lifecycleRefreshError(
          mutationError ?? new Error("The fork action completed."),
          refreshError,
        );
      }
      if (mutationError !== null) throw mutationError;
      throw new Error("The fork action returned no destination task.");
    },
    [mutationsReady, queryClient, scope],
  );

  const renameThread = React.useCallback(
    async (threadId: string, name: string) => {
      if (!mutationsReady) {
        throw new Error("Code task actions are unavailable while syncing.");
      }
      const renamed = await codeWorkspaceApi.renameCodeThread({
        scope,
        threadId,
        name,
      });
      if (renamed.id !== threadId) {
        throw new TypeError(
          "Renamed Code task identity did not match its request.",
        );
      }
      onThreadSnapshot(threadId, renamed);
      queryClient.setQueryData<CodeThreadsPage>(
        codeSessionQueryKeys.threads(scope),
        (current) =>
          current
            ? {
                ...current,
                data: current.data.map((row) =>
                  row.binding.codexThreadId === threadId
                    ? { ...row, thread: renamed, unavailable: null }
                    : row,
                ),
              }
            : current,
      );
      void queryClient.invalidateQueries({
        queryKey: codeSessionQueryKeys.threads(scope),
      });
    },
    [mutationsReady, onThreadSnapshot, queryClient, scope],
  );

  const acknowledgeAuthoritativeListRefresh = React.useCallback(() => {
    setUnreconciledThreadIds((current) =>
      current.size === 0 ? current : new Set(),
    );
  }, []);

  const acknowledgeAuthoritativeForkRefresh = React.useCallback(() => {
    const threads = queryClient.getQueryData<CodeThreadsPage>(
      codeSessionQueryKeys.threads(scope),
    );
    const preparations = queryClient.getQueryData<
      readonly CodeThreadPreparation[]
    >(codeSessionQueryKeys.preparations(scope));
    if (!threads || !preparations) return;
    setUnreconciledForkThreadIds((current) => {
      if (current.size === 0) return current;
      const next = new Set(current);
      for (const threadId of current) {
        const sourcePresent = threads.data.some(
          (row) => row.binding.codexThreadId === threadId,
        );
        const forkUnfinished = preparations.some(
          (preparation) =>
            preparation.operation === "fork" &&
            preparation.sourceThreadId === threadId,
        );
        if (
          sourcePresent &&
          !forkUnfinished &&
          !semanticForkBlockerThreadIdsRef.current.has(threadId)
        ) {
          next.delete(threadId);
        }
      }
      return next.size === current.size ? current : next;
    });
  }, [queryClient, scope]);

  const isLifecycleLocallyBlocked = React.useCallback(
    (threadId: string | null) =>
      threadId !== null &&
      (pendingLifecycleThreadId === threadId ||
        pendingForkThreadId === threadId ||
        unreconciledThreadIds.has(threadId) ||
        unreconciledForkThreadIds.has(threadId)),
    [
      pendingForkThreadId,
      pendingLifecycleThreadId,
      unreconciledForkThreadIds,
      unreconciledThreadIds,
    ],
  );

  const isForkLocallyBlocked = React.useCallback(
    (threadId: string) =>
      pendingLifecycleThreadId === threadId ||
      pendingForkThreadId === threadId ||
      unreconciledThreadIds.has(threadId) ||
      unreconciledForkThreadIds.has(threadId),
    [
      pendingForkThreadId,
      pendingLifecycleThreadId,
      unreconciledForkThreadIds,
      unreconciledThreadIds,
    ],
  );

  return {
    acknowledgeAuthoritativeForkRefresh,
    acknowledgeAuthoritativeListRefresh,
    archiveThread: (threadId: string) => mutateLifecycle(threadId, "archive"),
    forkThread,
    isForkLocallyBlocked,
    isLifecycleLocallyBlocked,
    pendingForkThreadId,
    pendingLifecycleThreadId,
    renameThread,
    unarchiveThread: (threadId: string) =>
      mutateLifecycle(threadId, "unarchive"),
  };
}
