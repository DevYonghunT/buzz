import { useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import {
  codeWorkspaceApi,
  getCodeThreadStartError,
} from "../api/codeWorkspace";
import type {
  CodeBoundThreadOpenResult,
  CodeModelSelection,
  CodeRepositoryDescriptor,
  CodeThreadBindingScope,
  CodeThreadPreparation,
  CodeThreadSummary,
  CodeThreadsPage,
} from "../api/types";
import {
  type CodeTimelineLocalPrompt,
  projectCodeTimeline,
} from "../lib/codeTimeline";
import {
  codeThreadLabel,
  codeThreadLifecycleCapabilities,
  selectCodeThreadId,
} from "../lib/codeWorkspaceView";
import {
  selectCanRespondToCodeApproval,
  selectCodePendingApprovals,
  selectCodeThreadEvents,
} from "../state/codeSessionReducer";
import {
  codeSessionQueryKeys,
  codeThreadPreparationsQueryOptions,
  codeThreadsQueryOptions,
} from "../state/codeSessionQueries";
import { useCodeSessionStore } from "../state/codeSessionStore";
import { useCodeModelSelection } from "../state/useCodeModelSelection";
import {
  type CodePendingTurn,
  useCodeSelectedTurn,
} from "../state/useCodeSelectedTurn";
import { useCodeThreadLifecycleSync } from "../state/useCodeThreadLifecycleSync";
import { useCodeThreadMutations } from "../state/useCodeThreadMutations";
import { useCodeWorkspaceGitHandoff } from "../state/useCodeWorkspaceGitHandoff";
import { CodeChangesPanel } from "./CodeChangesPanel";
import { CodeComposer } from "./CodeComposer";
import { CodeRuntimeStatus } from "./CodeRuntimeStatus";
import { CodeTerminalDrawer } from "./CodeTerminalDrawer";
import { CodeThreadSidebar } from "./CodeThreadSidebar";
import { CodeThreadLifecycleNotice } from "./CodeThreadLifecycleNotice";
import { CodeTimeline } from "./CodeTimeline";
import { CodeWorkspaceHeader } from "./CodeWorkspaceHeader";
import { useCodeTerminalVisibility } from "./useCodeTerminalVisibility";

function errorMessage(error: unknown): string {
  const threadStartError = getCodeThreadStartError(error);
  if (threadStartError) return threadStartError.message;
  return error instanceof Error ? error.message : String(error);
}

type OpenedThreads = ReadonlyMap<string, CodeBoundThreadOpenResult>;

export function CodeWorkspaceScreen({
  baseRef,
  onSelectedThreadIdChange,
  projectName,
  repository,
  scope,
  selectedThreadId,
}: {
  baseRef: string;
  onSelectedThreadIdChange: (
    threadId: string | null,
    replace?: boolean,
  ) => void;
  projectName: string;
  repository: CodeRepositoryDescriptor;
  scope: CodeThreadBindingScope;
  selectedThreadId: string | null;
}) {
  const queryClient = useQueryClient();
  const session = useCodeSessionStore(scope);
  const runtimeReady = session.state.runtimeStatus?.phase === "ready";
  const replayReady = session.state.replay.status === "synchronized";
  const interactionReady =
    runtimeReady &&
    replayReady &&
    !session.state.replay.needsAuthoritativeRefresh &&
    session.subscriptionError === null;
  const preparationsQuery = useQuery({
    ...codeThreadPreparationsQueryOptions(scope),
    refetchInterval: 5_000,
  });
  const threadsQuery = useQuery({
    ...codeThreadsQueryOptions(scope),
    enabled: runtimeReady,
  });
  const threads = React.useMemo(
    () => (runtimeReady ? (threadsQuery.data?.data ?? []) : []),
    [runtimeReady, threadsQuery.data?.data],
  );
  const threadListReady = threadsQuery.isSuccess && !threadsQuery.isFetching;
  const [openedThreads, setOpenedThreads] = React.useState<OpenedThreads>(
    () => new Map(),
  );
  const [resumingThreadId, setResumingThreadId] = React.useState<string | null>(
    null,
  );
  const [creating, setCreating] = React.useState(false);
  const [actionPendingId, setActionPendingId] = React.useState<string | null>(
    null,
  );
  const [actionError, setActionError] = React.useState<string | null>(null);
  const [sidebarOpen, setSidebarOpen] = React.useState(true);
  const [inspectorOpen, setInspectorOpen] = React.useState(
    () =>
      typeof window !== "undefined" &&
      window.matchMedia("(min-width: 1280px)").matches,
  );
  const [localPrompts, setLocalPrompts] = React.useState<
    ReadonlyMap<string, readonly CodeTimelineLocalPrompt[]>
  >(() => new Map());
  const [pendingTurns, setPendingTurns] = React.useState<
    ReadonlyMap<string, CodePendingTurn>
  >(() => new Map());
  const localPromptIdRef = React.useRef(0);
  const resumeAttemptedRef = React.useRef(new Set<string>());
  const resumeInFlightRef = React.useRef<string | null>(null);
  const authoritativeRefreshAttemptedRef = React.useRef(new Set<string>());
  const authoritativeRefreshDiscardedRef = React.useRef(new Set<string>());
  const mountedRef = React.useRef(true);
  const runtimeGenerationRef = React.useRef(session.state.runtimeGeneration);
  const subscriptionEpochRef = React.useRef(
    session.state.replay.subscriptionEpoch,
  );
  const selectedThreadIdRef = React.useRef(selectedThreadId);
  const previousRuntimeGenerationRef = React.useRef(
    session.state.runtimeGeneration,
  );
  runtimeGenerationRef.current = session.state.runtimeGeneration;
  subscriptionEpochRef.current = session.state.replay.subscriptionEpoch;
  selectedThreadIdRef.current = selectedThreadId;

  React.useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  React.useEffect(() => {
    const previousGeneration = previousRuntimeGenerationRef.current;
    const currentGeneration = session.state.runtimeGeneration;
    previousRuntimeGenerationRef.current = currentGeneration;
    if (
      previousGeneration === null ||
      currentGeneration === null ||
      previousGeneration === currentGeneration
    ) {
      return;
    }
    resumeAttemptedRef.current.clear();
    authoritativeRefreshAttemptedRef.current.clear();
    authoritativeRefreshDiscardedRef.current.clear();
    setOpenedThreads(new Map());
    setPendingTurns(new Map());
    setActionError(null);
  }, [session.state.runtimeGeneration]);

  React.useEffect(() => {
    if (!runtimeReady || threadsQuery.isPending || threadsQuery.isFetching) {
      return;
    }
    const resolved = selectCodeThreadId(selectedThreadId, threads);
    if (resolved !== selectedThreadId) {
      onSelectedThreadIdChange(resolved, true);
    }
  }, [
    onSelectedThreadIdChange,
    runtimeReady,
    selectedThreadId,
    threads,
    threadsQuery.isFetching,
    threadsQuery.isPending,
  ]);

  const selectedRow = React.useMemo(
    () =>
      threads.find(
        (thread) => thread.binding.codexThreadId === selectedThreadId,
      ) ?? null,
    [selectedThreadId, threads],
  );
  const openedThread = selectedThreadId
    ? (openedThreads.get(selectedThreadId) ?? null)
    : null;
  const selectedThread: CodeThreadSummary | null =
    openedThread?.thread ?? selectedRow?.thread ?? null;
  const modelSelection = useCodeModelSelection({
    openedThread,
    runtimeGeneration: session.state.runtimeGeneration,
    runtimeReady,
    selectedThreadId,
  });

  const refreshLists = React.useCallback(async () => {
    await Promise.all([
      queryClient.invalidateQueries({
        queryKey: codeSessionQueryKeys.preparations(scope),
      }),
      queryClient.invalidateQueries({
        queryKey: codeSessionQueryKeys.threads(scope),
      }),
      queryClient.invalidateQueries({
        queryKey: codeSessionQueryKeys.worktrees(scope),
      }),
    ]);
  }, [queryClient, scope]);

  const refreshAfterThreadOpen = React.useCallback(() => {
    void queryClient.invalidateQueries({
      queryKey: codeSessionQueryKeys.preparations(scope),
    });
    void queryClient.invalidateQueries({
      queryKey: codeSessionQueryKeys.worktrees(scope),
      refetchType: "none",
    });
  }, [queryClient, scope]);

  const retainOpenedThread = React.useCallback(
    (
      opened: CodeBoundThreadOpenResult,
      pendingSelection: CodeModelSelection | null = null,
    ) => {
      if (!mountedRef.current) return;
      void queryClient.cancelQueries({
        exact: true,
        queryKey: codeSessionQueryKeys.threads(scope),
      });
      modelSelection.seedOpenedThread(opened, pendingSelection);
      setOpenedThreads((current) => {
        const next = new Map(current);
        next.set(opened.thread.id, opened);
        return next;
      });
      queryClient.setQueryData<CodeThreadsPage>(
        codeSessionQueryKeys.threads(scope),
        (current) => ({
          data: [
            {
              binding: opened.binding,
              lifecycle: "active",
              thread: opened.thread,
              unavailable: null,
            },
            ...(current?.data.filter(
              (row) => row.binding.codexThreadId !== opened.thread.id,
            ) ?? []),
          ],
          nextCursor: current?.nextCursor ?? null,
          backwardsCursor: current?.backwardsCursor ?? null,
        }),
      );
      resumeAttemptedRef.current.add(opened.thread.id);
    },
    [modelSelection.seedOpenedThread, queryClient, scope],
  );

  const retainThreadSnapshot = React.useCallback(
    (threadId: string, thread: CodeThreadSummary) => {
      if (!mountedRef.current) return;
      setOpenedThreads((current) => {
        const opened = current.get(threadId);
        if (!opened) return current;
        const next = new Map(current);
        next.set(threadId, {
          ...opened,
          thread: { ...thread, turns: opened.thread.turns },
        });
        return next;
      });
    },
    [],
  );
  const threadMutations = useCodeThreadMutations({
    mutationsReady: interactionReady,
    onThreadSnapshot: retainThreadSnapshot,
    scope,
  });
  const taskActionPending =
    threadMutations.pendingForkThreadId !== null ||
    threadMutations.pendingLifecycleThreadId !== null;
  const sidebarActionsReady =
    interactionReady &&
    threadListReady &&
    !taskActionPending &&
    !creating &&
    actionPendingId === null &&
    !modelSelection.saving;
  const selectedCapabilities = selectedRow
    ? codeThreadLifecycleCapabilities(selectedRow.lifecycle)
    : null;
  const selectedLifecycleBlocked =
    threadMutations.isLifecycleLocallyBlocked(selectedThreadId);
  const selectedCanExecute =
    selectedCapabilities?.canExecute === true && !selectedLifecycleBlocked;
  const cachedRows = threadsQuery.data?.data;
  const terminalRow =
    selectedRow ??
    cachedRows?.find((row) => row.binding.codexThreadId === selectedThreadId);
  const terminalCanExecute = codeThreadLifecycleCapabilities(
    terminalRow?.lifecycle ?? "unknown",
  ).canExecute;
  const terminalThreadId =
    terminalCanExecute && !selectedLifecycleBlocked ? selectedThreadId : null;
  const terminal = useCodeTerminalVisibility(scope, terminalThreadId);
  useCodeThreadLifecycleSync({
    acknowledgeAuthoritativeForkRefresh:
      threadMutations.acknowledgeAuthoritativeForkRefresh,
    acknowledgeAuthoritativeListRefresh:
      threadMutations.acknowledgeAuthoritativeListRefresh,
    events: session.state.events,
    preparationsDataUpdatedAt: preparationsQuery.dataUpdatedAt,
    preparationsQuerySucceeded: preparationsQuery.isSuccess,
    queryClient,
    scope,
    threadsDataUpdatedAt: threadsQuery.dataUpdatedAt,
    threadsQuerySucceeded: threadsQuery.isSuccess,
  });

  React.useEffect(() => {
    if (selectedThreadId === null || selectedRow?.lifecycle === "active")
      return;
    resumeAttemptedRef.current.delete(selectedThreadId);
    setOpenedThreads((current) => {
      if (!current.has(selectedThreadId)) return current;
      const next = new Map(current);
      next.delete(selectedThreadId);
      return next;
    });
    setPendingTurns((current) => {
      if (!current.has(selectedThreadId)) return current;
      const next = new Map(current);
      next.delete(selectedThreadId);
      return next;
    });
  }, [selectedRow?.lifecycle, selectedThreadId]);

  const resumeThread = React.useCallback(
    async (threadId: string, force = false) => {
      if (!runtimeReady || (!force && openedThreads.has(threadId))) return;
      const runtimeGeneration = runtimeGenerationRef.current;
      if (runtimeGeneration === null) return;
      const row = threads.find(
        (thread) => thread.binding.codexThreadId === threadId,
      );
      if (
        !row ||
        row.unavailable ||
        !codeThreadLifecycleCapabilities(row.lifecycle).canExecute ||
        threadMutations.isLifecycleLocallyBlocked(threadId)
      ) {
        if (mountedRef.current) {
          setActionError(
            row?.unavailable ??
              "This task is not active and cannot be resumed.",
          );
        }
        return;
      }
      if (resumeInFlightRef.current !== null) return;
      const authoritativeRefresh =
        session.captureAuthoritativeRefreshCompletion();
      resumeAttemptedRef.current.add(threadId);
      resumeInFlightRef.current = threadId;
      setResumingThreadId(threadId);
      setActionError(null);
      try {
        const opened = await codeWorkspaceApi.resumeCodeThread({
          scope,
          threadId,
          model: null,
        });
        if (
          !mountedRef.current ||
          runtimeGenerationRef.current !== runtimeGeneration
        ) {
          return;
        }
        retainOpenedThread(opened);
        if (
          authoritativeRefresh !== null &&
          authoritativeRefresh.runtimeGeneration === runtimeGeneration &&
          subscriptionEpochRef.current ===
            authoritativeRefresh.subscriptionEpoch &&
          selectedThreadIdRef.current === threadId
        ) {
          if (authoritativeRefresh.complete()) {
            void queryClient.invalidateQueries({
              exact: true,
              queryKey: codeSessionQueryKeys.threadChanges({
                scope,
                threadId,
                runtimeGeneration,
              }),
            });
          }
        }
      } catch (error) {
        if (mountedRef.current) setActionError(errorMessage(error));
      } finally {
        if (resumeInFlightRef.current === threadId) {
          resumeInFlightRef.current = null;
        }
        if (mountedRef.current) setResumingThreadId(null);
      }
    },
    [
      openedThreads,
      queryClient,
      retainOpenedThread,
      runtimeReady,
      scope,
      session.captureAuthoritativeRefreshCompletion,
      threadMutations.isLifecycleLocallyBlocked,
      threads,
    ],
  );

  React.useEffect(() => {
    if (!session.state.replay.needsAuthoritativeRefresh) {
      authoritativeRefreshAttemptedRef.current.clear();
      authoritativeRefreshDiscardedRef.current.clear();
      return;
    }
    const runtimeGeneration = session.state.runtimeGeneration;
    const subscriptionEpoch = session.state.replay.subscriptionEpoch;
    if (
      !runtimeReady ||
      runtimeGeneration === null ||
      subscriptionEpoch === null
    ) {
      return;
    }
    const refreshIdentity = JSON.stringify([
      runtimeGeneration,
      subscriptionEpoch,
      selectedThreadId,
    ]);
    if (selectedThreadId === null || !selectedCanExecute) {
      if (authoritativeRefreshAttemptedRef.current.has(refreshIdentity)) return;
      const completion = session.captureAuthoritativeRefreshCompletion();
      if (
        completion?.runtimeGeneration === runtimeGeneration &&
        completion.subscriptionEpoch === subscriptionEpoch &&
        completion.complete()
      ) {
        authoritativeRefreshAttemptedRef.current.add(refreshIdentity);
      }
      return;
    }

    if (!authoritativeRefreshDiscardedRef.current.has(refreshIdentity)) {
      authoritativeRefreshDiscardedRef.current.add(refreshIdentity);
      resumeAttemptedRef.current.delete(selectedThreadId);
      setOpenedThreads((current) => {
        if (!current.has(selectedThreadId)) return current;
        const next = new Map(current);
        next.delete(selectedThreadId);
        return next;
      });
    }
    if (
      selectedRow === null ||
      resumingThreadId !== null ||
      resumeInFlightRef.current !== null ||
      authoritativeRefreshAttemptedRef.current.has(refreshIdentity)
    ) {
      return;
    }
    authoritativeRefreshAttemptedRef.current.add(refreshIdentity);
    void resumeThread(selectedThreadId, true);
  }, [
    resumeThread,
    resumingThreadId,
    runtimeReady,
    selectedCanExecute,
    selectedRow,
    selectedThreadId,
    session.captureAuthoritativeRefreshCompletion,
    session.state.replay.needsAuthoritativeRefresh,
    session.state.replay.subscriptionEpoch,
    session.state.runtimeGeneration,
  ]);

  React.useEffect(() => {
    if (
      !runtimeReady ||
      !selectedThreadId ||
      !selectedCanExecute ||
      session.state.replay.needsAuthoritativeRefresh ||
      resumingThreadId !== null ||
      resumeAttemptedRef.current.has(selectedThreadId)
    ) {
      return;
    }
    void resumeThread(selectedThreadId);
  }, [
    resumeThread,
    resumingThreadId,
    runtimeReady,
    selectedCanExecute,
    selectedThreadId,
    session.state.replay.needsAuthoritativeRefresh,
  ]);

  const openPreparation = React.useCallback(
    async (preparation: CodeThreadPreparation) => {
      if (
        !interactionReady ||
        !threadListReady ||
        creating ||
        actionPendingId !== null ||
        modelSelection.saving
      )
        return;
      const runtimeGeneration = runtimeGenerationRef.current;
      if (runtimeGeneration === null) return;
      const recovering =
        preparation.operation === "fork" || preparation.state === "starting";
      let threadStartPending = !recovering;
      setActionPendingId(preparation.preparationId);
      setActionError(null);
      try {
        const pendingSelection = recovering
          ? null
          : modelSelection.newThreadSelection;
        const input = {
          scope,
          preparationId: preparation.preparationId,
          model: pendingSelection?.model ?? null,
        };
        const opened = recovering
          ? await codeWorkspaceApi.recoverCodeThreadBinding({
              ...input,
              model: null,
            })
          : await codeWorkspaceApi.startCodeThread(input);
        threadStartPending = false;
        if (
          !mountedRef.current ||
          runtimeGenerationRef.current !== runtimeGeneration
        ) {
          return;
        }
        retainOpenedThread(opened, pendingSelection);
        onSelectedThreadIdChange(opened.thread.id);
        refreshAfterThreadOpen();
      } catch (error) {
        if (!mountedRef.current) return;
        if (threadStartPending) modelSelection.revalidateCatalog();
        setActionError(errorMessage(error));
        await refreshLists();
      } finally {
        if (mountedRef.current) setActionPendingId(null);
      }
    },
    [
      actionPendingId,
      creating,
      interactionReady,
      modelSelection.newThreadSelection,
      modelSelection.revalidateCatalog,
      modelSelection.saving,
      onSelectedThreadIdChange,
      refreshLists,
      refreshAfterThreadOpen,
      retainOpenedThread,
      scope,
      threadListReady,
    ],
  );

  const forkThread = React.useCallback(
    async (threadId: string) => {
      if (!threadListReady) return;
      const runtimeGeneration = runtimeGenerationRef.current;
      if (runtimeGeneration === null) return;
      const opened = await threadMutations.forkThread(threadId);
      if (
        !mountedRef.current ||
        runtimeGenerationRef.current !== runtimeGeneration
      )
        return;
      retainOpenedThread(opened);
      onSelectedThreadIdChange(opened.thread.id);
    },
    [
      onSelectedThreadIdChange,
      retainOpenedThread,
      threadListReady,
      threadMutations.forkThread,
    ],
  );

  const createTask = React.useCallback(async () => {
    if (
      !interactionReady ||
      !threadListReady ||
      creating ||
      actionPendingId !== null ||
      modelSelection.saving
    )
      return;
    const runtimeGeneration = runtimeGenerationRef.current;
    if (runtimeGeneration === null) return;
    setCreating(true);
    setActionError(null);
    const pendingSelection = modelSelection.newThreadSelection;
    let threadStartAttempted = false;
    try {
      const prepared = await codeWorkspaceApi.prepareCodeWorktree({
        scope,
        repositoryRoot: repository.repositoryRoot,
        baseRef,
        executionMode: "worktree",
      });
      if (
        !mountedRef.current ||
        runtimeGenerationRef.current !== runtimeGeneration
      ) {
        return;
      }
      threadStartAttempted = true;
      const opened = await codeWorkspaceApi.startCodeThread({
        scope,
        preparationId: prepared.preparationId,
        model: pendingSelection?.model ?? null,
      });
      threadStartAttempted = false;
      if (
        !mountedRef.current ||
        runtimeGenerationRef.current !== runtimeGeneration
      ) {
        return;
      }
      retainOpenedThread(opened, pendingSelection);
      onSelectedThreadIdChange(opened.thread.id);
      refreshAfterThreadOpen();
    } catch (error) {
      if (!mountedRef.current) return;
      if (threadStartAttempted) modelSelection.revalidateCatalog();
      setActionError(errorMessage(error));
      await refreshLists();
    } finally {
      if (mountedRef.current) setCreating(false);
    }
  }, [
    baseRef,
    creating,
    actionPendingId,
    interactionReady,
    modelSelection.newThreadSelection,
    modelSelection.revalidateCatalog,
    modelSelection.saving,
    onSelectedThreadIdChange,
    refreshLists,
    refreshAfterThreadOpen,
    repository.repositoryRoot,
    retainOpenedThread,
    scope,
    threadListReady,
  ]);

  const selectedThreadEvents = React.useMemo(
    () =>
      selectedThreadId
        ? selectCodeThreadEvents(session.state, selectedThreadId)
        : [],
    [selectedThreadId, session.state],
  );
  const changesRuntimeGeneration = session.state.runtimeGeneration;
  const { changesEnabled, controller: gitHandoff } = useCodeWorkspaceGitHandoff(
    {
      interactionReady,
      lifecycleBlocked: selectedLifecycleBlocked,
      runtimeGeneration: changesRuntimeGeneration,
      runtimeReady,
      scope,
      selectedRow,
      selectedThreadEvents,
    },
  );
  const { activeTurn, effectiveTurnId } = useCodeSelectedTurn({
    pendingTurns,
    runtimeReady,
    selectedThreadEvents,
    selectedThreadId,
    sessionState: session.state,
    setPendingTurns,
  });
  const pendingApprovals = selectedThreadId
    ? selectCodePendingApprovals(session.state, selectedThreadId)
    : [];
  const timelineRows = React.useMemo(
    () =>
      selectedThread
        ? projectCodeTimeline(
            selectedThread,
            selectedThreadEvents,
            localPrompts.get(selectedThread.id) ?? [],
          )
        : [],
    [localPrompts, selectedThread, selectedThreadEvents],
  );
  const submitPrompt = React.useCallback(
    async (prompt: string) => {
      if (
        !interactionReady ||
        modelSelection.saving ||
        !selectedCanExecute ||
        !selectedThreadId ||
        !openedThread
      ) {
        return false;
      }
      const runtimeGeneration = session.state.runtimeGeneration;
      if (runtimeGeneration === null) return false;
      let turnStartPending = effectiveTurnId === null;
      setActionError(null);
      try {
        const turn = effectiveTurnId
          ? await codeWorkspaceApi.steerCodeTurn({
              scope,
              threadId: selectedThreadId,
              expectedTurnId: effectiveTurnId,
              prompt,
            })
          : await codeWorkspaceApi.startCodeTurn({
              scope,
              threadId: selectedThreadId,
              prompt,
              model: modelSelection.turnSelection?.model ?? null,
              effort: modelSelection.turnSelection?.reasoningEffort ?? null,
            });
        turnStartPending = false;
        if (
          !mountedRef.current ||
          runtimeGenerationRef.current !== runtimeGeneration
        ) {
          return false;
        }
        if (effectiveTurnId === null) {
          setPendingTurns((current) => {
            const next = new Map(current);
            next.set(selectedThreadId, {
              runtimeGeneration,
              turnId: turn.id,
            });
            return next;
          });
        }
        const localPrompt: CodeTimelineLocalPrompt = {
          id: `local-prompt-${++localPromptIdRef.current}`,
          text: prompt,
          turnId: turn.id,
        };
        setLocalPrompts((current) => {
          const next = new Map(current);
          next.set(selectedThreadId, [
            ...(current.get(selectedThreadId) ?? []),
            localPrompt,
          ]);
          return next;
        });
        return true;
      } catch (error) {
        if (mountedRef.current) setActionError(errorMessage(error));
        if (turnStartPending) modelSelection.revalidateCatalog();
        return false;
      }
    },
    [
      effectiveTurnId,
      interactionReady,
      modelSelection.saving,
      modelSelection.revalidateCatalog,
      modelSelection.turnSelection,
      openedThread,
      selectedCanExecute,
      scope,
      selectedThreadId,
      session.state.runtimeGeneration,
    ],
  );

  const interrupt = React.useCallback(async () => {
    if (!selectedThreadId || !selectedCanExecute) return;
    setActionError(null);
    try {
      if (!(await session.interruptThread(selectedThreadId))) {
        throw new Error("The active turn changed before it could be stopped.");
      }
    } catch (error) {
      if (mountedRef.current) setActionError(errorMessage(error));
    }
  }, [selectedCanExecute, selectedThreadId, session.interruptThread]);

  const selectedLabel = selectedRow
    ? codeThreadLabel(selectedRow)
    : selectedThread?.name || selectedThread?.preview || "Code task";
  const selectionLoading =
    selectedThreadId !== null && resumingThreadId === selectedThreadId;
  const composerDisabled =
    !interactionReady ||
    !selectedCanExecute ||
    !selectedThreadId ||
    !openedThread ||
    selectionLoading ||
    modelSelection.saving ||
    gitHandoff.gitBlockerReason !== null;
  const modelSelectionDisabled =
    !interactionReady ||
    creating ||
    actionPendingId !== null ||
    selectionLoading ||
    effectiveTurnId !== null ||
    (selectedThreadId !== null && !selectedCanExecute);
  const scopedListError = preparationsQuery.error ?? threadsQuery.error;
  const visibleError =
    actionError ?? (scopedListError ? errorMessage(scopedListError) : null);

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden bg-background">
      <CodeRuntimeStatus
        error={session.runtimeError}
        onRefresh={() => void session.refreshRuntime()}
        onRetrySync={session.retryEventSync}
        onStart={() => {
          void session.startRuntime().catch(() => {
            // Runtime error copy is rendered inline by the store.
          });
        }}
        pending={session.runtimePending}
        replay={session.state.replay}
        status={session.state.runtimeStatus}
        subscriptionError={session.subscriptionError}
      />
      <div className="relative flex min-h-0 flex-1 overflow-hidden">
        {sidebarOpen ? (
          <CodeThreadSidebar
            actionPendingId={actionPendingId}
            actionsReady={sidebarActionsReady}
            canCreate={sidebarActionsReady}
            creating={creating}
            isForkBlocked={threadMutations.isForkLocallyBlocked}
            isLifecycleBlocked={threadMutations.isLifecycleLocallyBlocked}
            loading={preparationsQuery.isFetching || threadsQuery.isFetching}
            onArchiveThread={threadMutations.archiveThread}
            onCreate={() => void createTask()}
            onForkThread={forkThread}
            onOpenPreparation={(preparation) =>
              void openPreparation(preparation)
            }
            onRefresh={() => {
              void refreshLists();
            }}
            onRenameThread={threadMutations.renameThread}
            onSelectThread={(threadId) => {
              onSelectedThreadIdChange(threadId);
            }}
            onUnarchiveThread={threadMutations.unarchiveThread}
            preparations={preparationsQuery.data ?? []}
            scope={scope}
            selectedThreadId={selectedThreadId}
            threads={threads}
          />
        ) : null}
        <section className="flex min-h-0 min-w-0 flex-1 flex-col">
          <CodeWorkspaceHeader
            canReadChanges={selectedCapabilities?.canReadChanges === true}
            inspectorOpen={inspectorOpen}
            modelSelection={modelSelection}
            modelSelectionDisabled={modelSelectionDisabled}
            onInspectorOpenChange={setInspectorOpen}
            onRetry={() => {
              if (selectedThreadId) void resumeThread(selectedThreadId, true);
            }}
            onSidebarOpenChange={setSidebarOpen}
            onTerminalToggle={terminal.toggle}
            selectedLabel={selectedLabel}
            selectedRow={selectedRow}
            selectedThreadId={selectedThreadId}
            selectionLoading={selectionLoading}
            showRetry={
              selectedCanExecute &&
              selectedThreadId !== null &&
              actionError !== null &&
              openedThread === null
            }
            sidebarOpen={sidebarOpen}
            terminalOpen={terminal.open}
            terminalVisible={terminalThreadId !== null}
          />
          {visibleError ? (
            <div
              className="border-destructive/30 border-b bg-destructive/10 px-3 py-2 text-xs text-destructive"
              role="alert"
            >
              {visibleError}
            </div>
          ) : null}
          {selectedThread ? (
            <>
              <CodeTimeline
                approvals={pendingApprovals}
                canRespond={(approval) =>
                  selectedCanExecute &&
                  interactionReady &&
                  selectCanRespondToCodeApproval(session.state, approval)
                }
                loading={selectionLoading}
                onRespond={async (approval, response) => {
                  if (!(await session.respondToApproval(approval, response))) {
                    throw new Error(
                      "This approval changed before the response was sent.",
                    );
                  }
                }}
                rows={timelineRows}
              />
              {selectedRow?.lifecycle === "active" ? (
                <CodeComposer
                  active={effectiveTurnId !== null}
                  canInterrupt={
                    selectedCanExecute &&
                    interactionReady &&
                    activeTurn !== null
                  }
                  disabled={composerDisabled}
                  disabledReason={gitHandoff.gitBlockerReason}
                  onInterrupt={interrupt}
                  onSubmit={submitPrompt}
                />
              ) : selectedRow ? (
                <CodeThreadLifecycleNotice
                  blocked={selectedLifecycleBlocked}
                  lifecycle={selectedRow.lifecycle}
                  onRefresh={refreshLists}
                  onUnarchive={() =>
                    threadMutations.unarchiveThread(
                      selectedRow.binding.codexThreadId,
                    )
                  }
                />
              ) : null}
            </>
          ) : selectedRow?.unavailable ? (
            <div className="flex flex-1 items-center justify-center px-6 text-center text-sm text-muted-foreground">
              {selectedRow.unavailable}
            </div>
          ) : (
            <div className="flex flex-1 flex-col items-center justify-center px-6 text-center">
              <p className="text-sm font-medium">{projectName} Code</p>
              <p className="mt-1 max-w-sm text-xs text-muted-foreground">
                Create a task in an isolated worktree or choose a recent task.
              </p>
            </div>
          )}
        </section>
        {inspectorOpen &&
        selectedRow &&
        selectedCapabilities?.canReadChanges &&
        changesRuntimeGeneration !== null ? (
          <CodeChangesPanel
            binding={selectedRow.binding}
            className="absolute inset-y-0 right-0 z-20 w-[min(34rem,calc(100%-3rem))] shadow-xl xl:static xl:z-auto xl:w-[34rem] xl:max-w-[42vw] xl:shadow-none"
            controller={gitHandoff}
            enabled={changesEnabled}
            onClose={() => setInspectorOpen(false)}
            runtimeGeneration={changesRuntimeGeneration}
            scope={scope}
          />
        ) : null}
      </div>
      {terminalThreadId ? (
        <CodeTerminalDrawer
          key={`${scope.communityId}:${scope.projectDtag}:${scope.repositoryIdentity}:${terminalThreadId}`}
          onOpenChange={terminal.setOpen}
          open={terminal.open}
          scope={scope}
          threadId={terminalThreadId}
        />
      ) : null}
    </div>
  );
}
