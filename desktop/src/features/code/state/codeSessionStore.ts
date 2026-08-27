import { useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { codeWorkspaceApi, type CodeWorkspaceApi } from "../api/codeWorkspace";
import type {
  CodeApprovalResponse,
  CodeRuntimeProbe,
  CodeRuntimeStatus,
  CodeThreadBindingScope,
} from "../api/types";
import {
  codeSessionReducer,
  createCodeSessionState,
  type CodePendingApproval,
  type CodeSessionAction,
  type CodeSessionState,
  selectCanRespondToCodeApproval,
  selectCodeActiveTurns,
  selectCodeRuntimeEventsInput,
} from "./codeSessionReducer";
import {
  codeRuntimeStatusQueryOptions,
  codeSessionQueryKeys,
} from "./codeSessionQueries";

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function isAbortError(error: unknown): boolean {
  return error instanceof Error && error.name === "AbortError";
}

/** One fail-closed completion tied to an exact replay generation and epoch. */
export type CodeAuthoritativeRefreshCompletion = {
  readonly runtimeGeneration: number;
  readonly subscriptionEpoch: number;
  complete: () => boolean;
};

/** Re-run executable discovery before reading the lifecycle it updates. */
export async function reprobeCodeRuntimeStatus(
  api: Pick<CodeWorkspaceApi, "probeCodeRuntime" | "getCodeRuntimeStatus">,
): Promise<{
  probe: CodeRuntimeProbe;
  status: CodeRuntimeStatus;
}> {
  const probe = await api.probeCodeRuntime();
  const status = await api.getCodeRuntimeStatus();
  return { probe, status };
}

/** Capture a one-shot reducer completion that rejects any later state drift. */
export function captureCodeAuthoritativeRefreshCompletion(
  getState: () => CodeSessionState,
  dispatch: (action: CodeSessionAction) => void,
): CodeAuthoritativeRefreshCompletion | null {
  const current = getState();
  const runtimeGeneration = current.runtimeGeneration;
  const subscriptionEpoch = current.replay.subscriptionEpoch;
  if (
    runtimeGeneration === null ||
    subscriptionEpoch === null ||
    !current.replay.needsAuthoritativeRefresh ||
    current.runtimeStatus?.phase !== "ready" ||
    current.runtimeStatus.generation !== runtimeGeneration
  ) {
    return null;
  }

  let completed = false;
  return {
    runtimeGeneration,
    subscriptionEpoch,
    complete: () => {
      if (completed) return false;
      const latest = getState();
      if (
        latest.runtimeGeneration !== runtimeGeneration ||
        latest.replay.subscriptionEpoch !== subscriptionEpoch ||
        !latest.replay.needsAuthoritativeRefresh ||
        latest.runtimeStatus?.phase !== "ready" ||
        latest.runtimeStatus.generation !== runtimeGeneration
      ) {
        return false;
      }
      completed = true;
      dispatch({
        type: "authoritativeRefreshCompleted",
        runtimeGeneration,
        subscriptionEpoch,
      });
      return true;
    },
  };
}

/** Scope-owned reducer plus the one race-free native event subscription. */
export function useCodeSessionStore(
  scope: CodeThreadBindingScope,
  api: CodeWorkspaceApi = codeWorkspaceApi,
): {
  state: CodeSessionState;
  runtimePending: boolean;
  runtimeError: string | null;
  subscriptionError: string | null;
  startRuntime: () => Promise<CodeRuntimeStatus>;
  refreshRuntime: () => Promise<void>;
  retryEventSync: () => void;
  captureAuthoritativeRefreshCompletion: () => CodeAuthoritativeRefreshCompletion | null;
  respondToApproval: (
    approval: CodePendingApproval,
    response: CodeApprovalResponse,
  ) => Promise<boolean>;
  interruptThread: (threadId: string) => Promise<boolean>;
} {
  const sessionScope = React.useMemo(
    () => ({
      communityId: scope.communityId,
      projectDtag: scope.projectDtag,
      repositoryIdentity: scope.repositoryIdentity,
    }),
    [scope.communityId, scope.projectDtag, scope.repositoryIdentity],
  );
  const [state, dispatch] = React.useReducer(
    codeSessionReducer,
    sessionScope,
    createCodeSessionState,
  );
  const stateRef = React.useRef(state);
  stateRef.current = state;

  const queryClient = useQueryClient();
  const runtimeRevisionRef = React.useRef(0);
  const receivedRuntimeRevisionRef = React.useRef(0);
  const subscriptionEpochRef = React.useRef(0);
  const autoStartAttemptedRef = React.useRef(false);
  const fullReplayRequestedRef = React.useRef(false);
  const [runtimeMutationPending, setRuntimeMutationPending] =
    React.useState(false);
  const [runtimeMutationError, setRuntimeMutationError] = React.useState<
    string | null
  >(null);
  const [subscriptionError, setSubscriptionError] = React.useState<
    string | null
  >(null);
  const subscriptionErrorRef = React.useRef<string | null>(null);
  const [subscriptionRefresh, setSubscriptionRefresh] = React.useState(0);
  const replayRestartKey = `${state.runtimeGeneration ?? "none"}:${subscriptionRefresh}`;
  const activeReplayRestartKeyRef = React.useRef(replayRestartKey);

  const receiveRuntimeStatus = React.useCallback(
    (status: CodeRuntimeStatus, revision: number) => {
      if (revision <= receivedRuntimeRevisionRef.current) return;
      receivedRuntimeRevisionRef.current = revision;
      dispatch({ type: "runtimeStatusReceived", revision, status });
      queryClient.setQueryData(codeSessionQueryKeys.runtimeStatus(), status);
      if (status.phase === "ready") setRuntimeMutationError(null);
    },
    [queryClient],
  );

  const runtimeQuery = useQuery({
    ...codeRuntimeStatusQueryOptions(api),
    queryFn: async () => {
      const revision = ++runtimeRevisionRef.current;
      const status = await api.getCodeRuntimeStatus();
      receiveRuntimeStatus(status, revision);
      return status;
    },
    refetchInterval: 3_000,
  });
  const startRuntime = React.useCallback(async () => {
    const revision = ++runtimeRevisionRef.current;
    setRuntimeMutationPending(true);
    setRuntimeMutationError(null);
    try {
      const status = await api.startCodeRuntime();
      receiveRuntimeStatus(status, revision);
      return status;
    } catch (error) {
      setRuntimeMutationError(errorMessage(error));
      throw error;
    } finally {
      setRuntimeMutationPending(false);
    }
  }, [api, receiveRuntimeStatus]);

  React.useEffect(() => {
    if (
      state.runtimeStatus?.phase !== "stopped" ||
      autoStartAttemptedRef.current ||
      runtimeMutationPending
    ) {
      return;
    }
    autoStartAttemptedRef.current = true;
    void startRuntime().catch(() => {
      // The inline runtime state owns retry guidance.
    });
  }, [runtimeMutationPending, startRuntime, state.runtimeStatus?.phase]);

  React.useEffect(() => {
    if (state.runtimeStatus?.phase !== "ready") {
      subscriptionErrorRef.current = null;
      setSubscriptionError(null);
      return;
    }

    const controller = new AbortController();
    const current = stateRef.current;
    const input = selectCodeRuntimeEventsInput(
      current,
      fullReplayRequestedRef.current,
    );
    fullReplayRequestedRef.current = false;
    const subscriptionEpoch = ++subscriptionEpochRef.current;
    let disposed = false;
    let unlisten: (() => void) | null = null;
    activeReplayRestartKeyRef.current = replayRestartKey;
    subscriptionErrorRef.current = null;
    setSubscriptionError(null);
    dispatch({ type: "subscriptionStarted", subscriptionEpoch, input });

    void api
      .listenAndReplayCodeWorkspaceEvents(
        input,
        {
          onReplay: (batch) => dispatch({ type: "replayReceived", batch }),
          onEvent: (event, epoch) =>
            dispatch({
              type: "eventReceived",
              subscriptionEpoch: epoch,
              event,
            }),
          onError: (error) => {
            if (
              !disposed &&
              subscriptionEpochRef.current === subscriptionEpoch &&
              activeReplayRestartKeyRef.current === replayRestartKey
            ) {
              const message = errorMessage(error);
              subscriptionErrorRef.current = message;
              setSubscriptionError(message);
            }
          },
        },
        { scope: sessionScope, subscriptionEpoch, signal: controller.signal },
      )
      .then((dispose) => {
        if (disposed) {
          dispose();
        } else {
          unlisten = dispose;
        }
      })
      .catch((error: unknown) => {
        if (
          !disposed &&
          subscriptionEpochRef.current === subscriptionEpoch &&
          activeReplayRestartKeyRef.current === replayRestartKey &&
          !isAbortError(error)
        ) {
          const message = errorMessage(error);
          subscriptionErrorRef.current = message;
          setSubscriptionError(message);
        }
      });

    return () => {
      disposed = true;
      controller.abort();
      unlisten?.();
    };
  }, [api, replayRestartKey, sessionScope, state.runtimeStatus?.phase]);

  const refreshRuntime = React.useCallback(async () => {
    setRuntimeMutationPending(true);
    setRuntimeMutationError(null);
    try {
      const { probe, status } = await reprobeCodeRuntimeStatus(api);
      queryClient.setQueryData(codeSessionQueryKeys.runtimeProbe(), probe);
      receiveRuntimeStatus(status, ++runtimeRevisionRef.current);
    } catch (error) {
      setRuntimeMutationError(errorMessage(error));
    } finally {
      setRuntimeMutationPending(false);
    }
  }, [api, queryClient, receiveRuntimeStatus]);

  const retryEventSync = React.useCallback(() => {
    fullReplayRequestedRef.current = true;
    setSubscriptionRefresh((value) => value + 1);
  }, []);

  const captureAuthoritativeRefreshCompletion = React.useCallback(
    () =>
      captureCodeAuthoritativeRefreshCompletion(
        () => stateRef.current,
        dispatch,
      ),
    [],
  );

  const respondToApproval = React.useCallback(
    async (approval: CodePendingApproval, response: CodeApprovalResponse) => {
      const current = stateRef.current;
      if (
        current.replay.status !== "synchronized" ||
        current.replay.needsAuthoritativeRefresh ||
        current.replay.approvalStateIncomplete ||
        subscriptionErrorRef.current !== null ||
        !selectCanRespondToCodeApproval(current, approval)
      ) {
        return false;
      }
      const input = {
        runtimeGeneration: approval.runtimeGeneration,
        requestId: approval.requestId,
        scope: approval.scope,
        threadId: approval.threadId,
        turnId: approval.turnId,
        response,
      };
      await api.respondToCodeApproval(input);
      dispatch({
        type: "approvalResponseCommitted",
        input,
        expectedSequence: approval.sequence,
        expectedItemId: approval.itemId,
      });
      return true;
    },
    [api],
  );

  const interruptThread = React.useCallback(
    async (threadId: string) => {
      const current = stateRef.current;
      const turn = selectCodeActiveTurns(current, threadId).at(-1);
      if (
        !turn ||
        current.replay.status !== "synchronized" ||
        current.replay.needsAuthoritativeRefresh ||
        current.replay.approvalStateIncomplete ||
        subscriptionErrorRef.current !== null ||
        current.runtimeStatus?.phase !== "ready" ||
        current.runtimeStatus.generation !== turn.runtimeGeneration
      ) {
        return false;
      }
      const input = { scope: current.scope, threadId, turnId: turn.turnId };
      await api.interruptCodeTurn(input);
      dispatch({
        type: "turnInterruptCommitted",
        runtimeGeneration: turn.runtimeGeneration,
        input,
      });
      return true;
    },
    [api],
  );

  return {
    state,
    runtimePending: runtimeQuery.isPending || runtimeMutationPending,
    runtimeError:
      runtimeMutationError ??
      (runtimeQuery.error ? errorMessage(runtimeQuery.error) : null),
    subscriptionError,
    startRuntime,
    refreshRuntime,
    retryEventSync,
    captureAuthoritativeRefreshCompletion,
    respondToApproval,
    interruptThread,
  };
}
