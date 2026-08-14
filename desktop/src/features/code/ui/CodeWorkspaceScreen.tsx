import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  GitBranch,
  LoaderCircle,
  PanelLeftClose,
  PanelLeftOpen,
} from "lucide-react";
import * as React from "react";

import {
  codeWorkspaceApi,
  getCodeThreadStartError,
} from "../api/codeWorkspace";
import type {
  CodeBoundThreadOpenResult,
  CodeRepositoryDescriptor,
  CodeThreadBindingScope,
  CodeThreadPreparation,
  CodeThreadSummary,
} from "../api/types";
import {
  type CodeTimelineLocalPrompt,
  projectCodeTimeline,
} from "../lib/codeTimeline";
import { codeThreadLabel, selectCodeThreadId } from "../lib/codeWorkspaceView";
import {
  selectCanRespondToCodeApproval,
  selectCodeActiveTurns,
  selectCodePendingApprovals,
  selectCodeThreadEvents,
} from "../state/codeSessionReducer";
import {
  codeSessionQueryKeys,
  codeThreadPreparationsQueryOptions,
  codeThreadsQueryOptions,
} from "../state/codeSessionQueries";
import { useCodeSessionStore } from "../state/codeSessionStore";
import { Button } from "@/shared/ui/button";
import { CodeComposer } from "./CodeComposer";
import { CodeRuntimeStatus } from "./CodeRuntimeStatus";
import { CodeThreadSidebar } from "./CodeThreadSidebar";
import { CodeTimeline } from "./CodeTimeline";

function errorMessage(error: unknown): string {
  const threadStartError = getCodeThreadStartError(error);
  if (threadStartError) return threadStartError.message;
  return error instanceof Error ? error.message : String(error);
}

type OpenedThreads = ReadonlyMap<string, CodeBoundThreadOpenResult>;
type PendingTurn = {
  readonly runtimeGeneration: number;
  readonly turnId: string;
};

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
    runtimeReady && replayReady && session.subscriptionError === null;
  const preparationsQuery = useQuery({
    ...codeThreadPreparationsQueryOptions(scope),
    refetchInterval: 5_000,
  });
  const threadsQuery = useQuery({
    ...codeThreadsQueryOptions(scope),
    enabled: runtimeReady,
    refetchInterval: runtimeReady ? 5_000 : false,
  });
  const threads = React.useMemo(
    () => (runtimeReady ? (threadsQuery.data?.data ?? []) : []),
    [runtimeReady, threadsQuery.data?.data],
  );
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
  const [localPrompts, setLocalPrompts] = React.useState<
    ReadonlyMap<string, readonly CodeTimelineLocalPrompt[]>
  >(() => new Map());
  const [pendingTurns, setPendingTurns] = React.useState<
    ReadonlyMap<string, PendingTurn>
  >(() => new Map());
  const localPromptIdRef = React.useRef(0);
  const resumeAttemptedRef = React.useRef(new Set<string>());
  const resumeInFlightRef = React.useRef<string | null>(null);
  const mountedRef = React.useRef(true);
  const runtimeGenerationRef = React.useRef(session.state.runtimeGeneration);
  const previousRuntimeGenerationRef = React.useRef(
    session.state.runtimeGeneration,
  );
  runtimeGenerationRef.current = session.state.runtimeGeneration;

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

  const refreshLists = React.useCallback(async () => {
    await Promise.all([
      queryClient.invalidateQueries({
        queryKey: codeSessionQueryKeys.preparations(scope),
      }),
      queryClient.invalidateQueries({
        queryKey: codeSessionQueryKeys.threads(scope),
      }),
    ]);
  }, [queryClient, scope]);

  const retainOpenedThread = React.useCallback(
    (opened: CodeBoundThreadOpenResult) => {
      if (!mountedRef.current) return;
      setOpenedThreads((current) => {
        const next = new Map(current);
        next.set(opened.thread.id, opened);
        return next;
      });
      resumeAttemptedRef.current.add(opened.thread.id);
    },
    [],
  );

  const resumeThread = React.useCallback(
    async (threadId: string, force = false) => {
      if (!runtimeReady || (!force && openedThreads.has(threadId))) return;
      const runtimeGeneration = runtimeGenerationRef.current;
      if (runtimeGeneration === null) return;
      const row = threads.find(
        (thread) => thread.binding.codexThreadId === threadId,
      );
      if (!row || row.unavailable) {
        if (mountedRef.current) {
          setActionError(row?.unavailable ?? "This task is unavailable.");
        }
        return;
      }
      if (resumeInFlightRef.current !== null) return;
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
        if (runtimeGenerationRef.current !== runtimeGeneration) return;
        retainOpenedThread(opened);
      } catch (error) {
        if (mountedRef.current) setActionError(errorMessage(error));
      } finally {
        if (resumeInFlightRef.current === threadId) {
          resumeInFlightRef.current = null;
        }
        if (mountedRef.current) setResumingThreadId(null);
      }
    },
    [openedThreads, retainOpenedThread, runtimeReady, scope, threads],
  );

  React.useEffect(() => {
    if (
      !runtimeReady ||
      !selectedThreadId ||
      resumingThreadId !== null ||
      resumeAttemptedRef.current.has(selectedThreadId)
    ) {
      return;
    }
    void resumeThread(selectedThreadId);
  }, [resumeThread, resumingThreadId, runtimeReady, selectedThreadId]);

  const openPreparation = React.useCallback(
    async (preparation: CodeThreadPreparation) => {
      if (!interactionReady || creating || actionPendingId !== null) return;
      const runtimeGeneration = runtimeGenerationRef.current;
      if (runtimeGeneration === null) return;
      setActionPendingId(preparation.preparationId);
      setActionError(null);
      try {
        const input = {
          scope,
          preparationId: preparation.preparationId,
          model: null,
        };
        const opened =
          preparation.state === "starting"
            ? await codeWorkspaceApi.recoverCodeThreadBinding(input)
            : await codeWorkspaceApi.startCodeThread(input);
        if (
          !mountedRef.current ||
          runtimeGenerationRef.current !== runtimeGeneration
        ) {
          return;
        }
        retainOpenedThread(opened);
        await refreshLists();
        if (!mountedRef.current) return;
        onSelectedThreadIdChange(opened.thread.id);
      } catch (error) {
        if (!mountedRef.current) return;
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
      onSelectedThreadIdChange,
      refreshLists,
      retainOpenedThread,
      scope,
    ],
  );

  const createTask = React.useCallback(async () => {
    if (!interactionReady || creating || actionPendingId !== null) return;
    const runtimeGeneration = runtimeGenerationRef.current;
    if (runtimeGeneration === null) return;
    setCreating(true);
    setActionError(null);
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
      const opened = await codeWorkspaceApi.startCodeThread({
        scope,
        preparationId: prepared.preparationId,
        model: null,
      });
      if (
        !mountedRef.current ||
        runtimeGenerationRef.current !== runtimeGeneration
      ) {
        return;
      }
      retainOpenedThread(opened);
      await refreshLists();
      if (!mountedRef.current) return;
      onSelectedThreadIdChange(opened.thread.id);
    } catch (error) {
      if (!mountedRef.current) return;
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
    onSelectedThreadIdChange,
    refreshLists,
    repository.repositoryRoot,
    retainOpenedThread,
    scope,
  ]);

  const activeTurns = selectedThreadId
    ? selectCodeActiveTurns(session.state, selectedThreadId)
    : [];
  const activeTurn = activeTurns.at(-1) ?? null;
  const selectedThreadEvents = React.useMemo(
    () =>
      selectedThreadId
        ? selectCodeThreadEvents(session.state, selectedThreadId)
        : [],
    [selectedThreadId, session.state],
  );
  const selectedPendingTurn = selectedThreadId
    ? (pendingTurns.get(selectedThreadId) ?? null)
    : null;
  const selectedPendingTurnObserved =
    selectedPendingTurn !== null &&
    selectedThreadEvents.some(
      (event) =>
        event.runtimeGeneration === selectedPendingTurn.runtimeGeneration &&
        event.turnId === selectedPendingTurn.turnId &&
        (event.kind === "turn/started" || event.kind === "turn/completed"),
    );
  const pendingTurn =
    selectedPendingTurn !== null &&
    !selectedPendingTurnObserved &&
    runtimeReady &&
    session.state.runtimeGeneration === selectedPendingTurn.runtimeGeneration
      ? selectedPendingTurn
      : null;
  const effectiveTurnId = activeTurn?.turnId ?? pendingTurn?.turnId ?? null;
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

  React.useEffect(() => {
    if (pendingTurns.size === 0) return;
    setPendingTurns((current) => {
      let next: Map<string, PendingTurn> | null = null;
      for (const [threadId, pending] of current) {
        const generationStillReady =
          session.state.runtimeStatus?.phase === "ready" &&
          session.state.runtimeGeneration === pending.runtimeGeneration;
        const observedActiveTurn = selectCodeActiveTurns(
          session.state,
          threadId,
        ).some(
          (turn) =>
            turn.turnId === pending.turnId &&
            turn.runtimeGeneration === pending.runtimeGeneration,
        );
        const observedTurnEvent = session.state.events.some(
          (event) =>
            event.runtimeGeneration === pending.runtimeGeneration &&
            event.threadId === threadId &&
            event.turnId === pending.turnId &&
            (event.kind === "turn/started" || event.kind === "turn/completed"),
        );
        if (!generationStillReady || observedActiveTurn || observedTurnEvent) {
          next ??= new Map(current);
          next.delete(threadId);
        }
      }
      return next ?? current;
    });
  }, [pendingTurns.size, session.state]);

  const submitPrompt = React.useCallback(
    async (prompt: string) => {
      if (!interactionReady || !selectedThreadId || !openedThread) return false;
      const runtimeGeneration = session.state.runtimeGeneration;
      if (runtimeGeneration === null) return false;
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
              model: null,
              effort: null,
            });
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
        return false;
      }
    },
    [
      effectiveTurnId,
      interactionReady,
      openedThread,
      scope,
      selectedThreadId,
      session.state.runtimeGeneration,
    ],
  );

  const interrupt = React.useCallback(async () => {
    if (!selectedThreadId) return;
    setActionError(null);
    try {
      if (!(await session.interruptThread(selectedThreadId))) {
        throw new Error("The active turn changed before it could be stopped.");
      }
    } catch (error) {
      if (mountedRef.current) setActionError(errorMessage(error));
    }
  }, [selectedThreadId, session.interruptThread]);

  const selectedLabel = selectedRow
    ? codeThreadLabel(selectedRow)
    : selectedThread?.name || selectedThread?.preview || "Code task";
  const selectionLoading =
    selectedThreadId !== null && resumingThreadId === selectedThreadId;
  const composerDisabled =
    !interactionReady || !selectedThreadId || !openedThread || selectionLoading;
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
      <div className="flex min-h-0 flex-1 overflow-hidden">
        {sidebarOpen ? (
          <CodeThreadSidebar
            actionPendingId={actionPendingId}
            canCreate={interactionReady && actionPendingId === null}
            creating={creating}
            loading={preparationsQuery.isFetching || threadsQuery.isFetching}
            onCreate={() => void createTask()}
            onOpenPreparation={(preparation) =>
              void openPreparation(preparation)
            }
            onRefresh={() => {
              void refreshLists();
            }}
            onSelectThread={(threadId) => {
              onSelectedThreadIdChange(threadId);
            }}
            preparations={preparationsQuery.data ?? []}
            selectedThreadId={selectedThreadId}
            threads={threads}
          />
        ) : null}
        <section className="flex min-h-0 min-w-0 flex-1 flex-col">
          <header className="flex h-11 shrink-0 items-center gap-2 border-border/60 border-b px-3">
            <Button
              aria-label={
                sidebarOpen ? "Hide task sidebar" : "Show task sidebar"
              }
              className="h-7 w-7 shrink-0"
              onClick={() => setSidebarOpen((open) => !open)}
              size="icon-xs"
              title={sidebarOpen ? "Hide task sidebar" : "Show task sidebar"}
              variant="ghost"
            >
              {sidebarOpen ? <PanelLeftClose /> : <PanelLeftOpen />}
            </Button>
            <div className="min-w-0 flex-1">
              <h2 className="truncate text-sm font-semibold">
                {selectedThreadId ? selectedLabel : "Choose or create a task"}
              </h2>
              {selectedRow ? (
                <p className="flex min-w-0 items-center gap-1 text-2xs text-muted-foreground">
                  <GitBranch className="h-3 w-3 shrink-0" />
                  <span className="truncate">
                    {selectedRow.binding.executionMode === "worktree"
                      ? "Managed worktree"
                      : "Local checkout"}
                    {` · ${selectedRow.binding.baseRef.slice(0, 8)}`}
                  </span>
                </p>
              ) : null}
            </div>
            {selectionLoading ? (
              <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
                <LoaderCircle className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" />
                Resuming…
              </span>
            ) : null}
            {selectedThreadId && actionError && !openedThread ? (
              <Button
                onClick={() => void resumeThread(selectedThreadId, true)}
                size="xs"
                variant="outline"
              >
                Retry task
              </Button>
            ) : null}
          </header>
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
              <CodeComposer
                active={effectiveTurnId !== null}
                canInterrupt={interactionReady && activeTurn !== null}
                disabled={composerDisabled}
                onInterrupt={interrupt}
                onSubmit={submitPrompt}
              />
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
      </div>
    </div>
  );
}
