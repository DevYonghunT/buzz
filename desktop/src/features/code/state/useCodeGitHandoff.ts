import { useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { codeWorkspaceApi } from "../api/codeWorkspace";
import type {
  CodeGitChangeFile,
  CodeGitCommitReceipt,
  CodeGitIndexMutationReceipt,
  CodeGitMutationReceipt,
  CodeGitOperation,
  CodeGitReadyStatus,
  CodeGitReconcileResult,
  CodeGitStatus,
} from "../api/codeGitTypes";
import type { CodeThreadBindingScope } from "../api/types";
import {
  type CodeGitHandoffAttempt,
  codeGitReceiptsMatch,
  codeGitReconcileReceiptError,
  pollCodeGitReconcile,
  settleCodeGitReceipt,
} from "./codeGitHandoffMachine";
import {
  codeSessionQueryKeys,
  codeThreadGitAttemptQueryOptions,
  codeThreadGitStatusQueryOptions,
} from "./codeSessionQueries";

function messageFrom(error: unknown, fallback: string): string {
  return error instanceof Error && error.message.trim()
    ? error.message
    : fallback;
}

function maximumRevision(...values: Array<number | null | undefined>) {
  const revisions = values.filter((value): value is number => value != null);
  return revisions.length === 0 ? null : Math.max(...revisions);
}

function scopeMatches(
  left: CodeThreadBindingScope,
  right: CodeThreadBindingScope,
) {
  return (
    left.communityId === right.communityId &&
    left.projectDtag === right.projectDtag &&
    left.repositoryIdentity === right.repositoryIdentity
  );
}

function reconcileResultMatches(
  result: CodeGitReconcileResult,
  scope: CodeThreadBindingScope,
  threadId: string,
) {
  const identity = result.state === "completed" ? result.receipt : result;
  return identity.threadId === threadId && scopeMatches(identity.scope, scope);
}

function attemptMessage(attempt: CodeGitHandoffAttempt): string {
  if (attempt.message) return attempt.message;
  if (attempt.state === "pending") return `Applying ${attempt.label}…`;
  if (attempt.state === "refreshing") {
    return "Git write completed. Confirming authoritative status…";
  }
  return "Checking native Git operation status…";
}

function blockerReason({
  attempt,
  enabled,
  queryError,
  queryPending,
  status,
}: {
  attempt: CodeGitHandoffAttempt | null;
  enabled: boolean;
  queryError: boolean;
  queryPending: boolean;
  status: CodeGitStatus | undefined;
}): string | null {
  if (!enabled) return null;
  if (attempt !== null) return attemptMessage(attempt);
  if (queryError) {
    return "Git handoff status could not be verified. Open Changes and retry status before starting or steering a turn.";
  }
  if (queryPending) {
    return "Checking native Git handoff status before Code can continue…";
  }
  if (status?.state === "recoveryRequired") {
    return `The ${status.operation.operation} operation must be reconciled before Code can continue.`;
  }
  if (status?.state === "ready" && status.blockingReceipt !== null) {
    return `The completed ${status.blockingReceipt.operation} operation must be acknowledged before Code can continue.`;
  }
  return null;
}

function baseAttempt(
  operation: CodeGitOperation,
  label: string,
  ready: CodeGitReadyStatus,
): CodeGitHandoffAttempt {
  return {
    state: "pending",
    operation,
    operationId: null,
    label,
    requestGeneration: ready.writeGeneration,
    baselineStatusRevision: ready.statusRevision,
    receipt: null,
    message: null,
  };
}

export function useCodeGitHandoff({
  enabled,
  runtimeGeneration,
  scope,
  threadId,
}: {
  enabled: boolean;
  runtimeGeneration: number | null;
  scope: CodeThreadBindingScope;
  threadId: string | null;
}) {
  const queryClient = useQueryClient();
  const queryThreadId = threadId ?? "";
  const queryRuntimeGeneration = runtimeGeneration ?? 0;
  const queryEnabled =
    enabled && threadId !== null && runtimeGeneration !== null;
  const query = useQuery({
    ...codeThreadGitStatusQueryOptions({
      scope,
      threadId: queryThreadId,
      runtimeGeneration: queryRuntimeGeneration,
    }),
    enabled: queryEnabled,
    refetchOnMount: "always",
    staleTime: 1_000,
  });
  const attemptOptions = codeThreadGitAttemptQueryOptions({
    scope,
    threadId: queryThreadId,
  });
  const attemptQuery = useQuery(attemptOptions);
  const attempt = threadId === null ? null : (attemptQuery.data ?? null);
  const status = query.data;
  const ready: CodeGitReadyStatus | null =
    status?.state === "ready" ? status : null;
  const statusKey = React.useMemo(
    () =>
      codeSessionQueryKeys.threadGitStatus({
        scope,
        threadId: queryThreadId,
        runtimeGeneration: queryRuntimeGeneration,
      }),
    [queryRuntimeGeneration, queryThreadId, scope],
  );
  const attemptKey = React.useMemo(
    () =>
      codeSessionQueryKeys.threadGitAttempt({ scope, threadId: queryThreadId }),
    [queryThreadId, scope],
  );
  const inFlightRef = React.useRef(new Set<string>());

  const setAttempt = React.useCallback(
    (next: CodeGitHandoffAttempt | null) => {
      queryClient.setQueryData(attemptKey, next);
    },
    [attemptKey, queryClient],
  );

  const replaceStatus = React.useCallback(
    (next: CodeGitStatus) => {
      const current = queryClient.getQueryData<CodeGitStatus>(statusKey);
      if (
        current !== undefined &&
        (next.statusRevision < current.statusRevision ||
          (next.statusRevision === current.statusRevision &&
            next.state === "ready" &&
            current.state === "ready" &&
            (next.writeGeneration < current.writeGeneration ||
              (next.writeGeneration === current.writeGeneration &&
                next.snapshotSequence < current.snapshotSequence))))
      ) {
        throw new TypeError("Late Code Git status response was discarded");
      }
      queryClient.setQueryData(statusKey, next);
      return next;
    },
    [queryClient, statusKey],
  );

  const readRawStatus = React.useCallback(async () => {
    if (threadId === null || runtimeGeneration === null) {
      throw new TypeError("Code Git status has no selected runtime thread");
    }
    const next = await codeWorkspaceApi.getCodeThreadGitStatus({
      scope,
      threadId,
    });
    if (
      next.runtimeGeneration !== runtimeGeneration ||
      next.threadId !== threadId ||
      !scopeMatches(next.scope, scope)
    ) {
      throw new TypeError(
        "Code Git status did not match the selected runtime thread",
      );
    }
    return next;
  }, [runtimeGeneration, scope, threadId]);

  const settleReceipt = React.useCallback(
    async (
      receipt: CodeGitMutationReceipt,
      label: string,
      minimumStatusRevision: number | null,
    ) => {
      const context: CodeGitHandoffAttempt = {
        state: "refreshing",
        operation: receipt.operation,
        operationId: receipt.operationId,
        label,
        requestGeneration: receipt.requestGeneration,
        baselineStatusRevision: minimumStatusRevision,
        receipt,
        message: null,
      };
      setAttempt(context);
      const settlement = await settleCodeGitReceipt({
        acceptStatus: replaceStatus,
        receipt,
        minimumStatusRevision,
        readStatus: readRawStatus,
        acknowledge: async (blocking) => {
          await codeWorkspaceApi.acknowledgeCodeThreadGit({
            scope,
            threadId: receipt.threadId,
            operationId: receipt.operationId,
            writeGeneration: blocking.writeGeneration,
            snapshotId: blocking.snapshotId,
          });
        },
      });
      if (settlement.state === "settled") {
        setAttempt(null);
        return true;
      }
      setAttempt({
        ...context,
        state: "unknown",
        message: settlement.message,
      });
      return false;
    },
    [readRawStatus, replaceStatus, scope, setAttempt],
  );

  const runIndexMutation = React.useCallback(
    async (operation: "stage" | "unstage", file: CodeGitChangeFile) => {
      if (
        ready === null ||
        attempt !== null ||
        ready.blockingReceipt !== null ||
        !ready.capabilities[operation].enabled ||
        threadId === null ||
        inFlightRef.current.has(queryThreadId)
      ) {
        return;
      }
      inFlightRef.current.add(queryThreadId);
      await queryClient.cancelQueries({ exact: true, queryKey: statusKey });
      const context = baseAttempt(
        operation,
        `${operation} ${file.path}`,
        ready,
      );
      setAttempt(context);
      let receipt: CodeGitIndexMutationReceipt;
      try {
        const input = {
          scope,
          threadId,
          writeGeneration: ready.writeGeneration,
          snapshotId: ready.snapshotId,
          fileId: file.fileId,
        };
        receipt =
          operation === "stage"
            ? await codeWorkspaceApi.stageCodeThreadFile(input)
            : await codeWorkspaceApi.unstageCodeThreadFile(input);
      } catch (error) {
        setAttempt({
          ...context,
          state: "unknown",
          message: messageFrom(error, "Git operation outcome is unknown."),
        });
        inFlightRef.current.delete(queryThreadId);
        return;
      }
      await settleReceipt(
        receipt,
        context.label,
        context.baselineStatusRevision,
      );
      inFlightRef.current.delete(queryThreadId);
    },
    [
      attempt,
      queryClient,
      queryThreadId,
      ready,
      scope,
      setAttempt,
      settleReceipt,
      statusKey,
      threadId,
    ],
  );

  const commit = React.useCallback(
    async (message: string): Promise<CodeGitCommitReceipt | null> => {
      if (
        ready === null ||
        attempt !== null ||
        ready.blockingReceipt !== null ||
        !ready.capabilities.commit.enabled ||
        threadId === null ||
        inFlightRef.current.has(queryThreadId)
      ) {
        return null;
      }
      inFlightRef.current.add(queryThreadId);
      await queryClient.cancelQueries({ exact: true, queryKey: statusKey });
      const context = baseAttempt("commit", "commit", ready);
      setAttempt(context);
      let receipt: CodeGitCommitReceipt;
      try {
        receipt = await codeWorkspaceApi.commitCodeThread({
          scope,
          threadId,
          writeGeneration: ready.writeGeneration,
          snapshotId: ready.snapshotId,
          message,
        });
      } catch (error) {
        setAttempt({
          ...context,
          state: "unknown",
          message: messageFrom(error, "Commit outcome is unknown."),
        });
        inFlightRef.current.delete(queryThreadId);
        return null;
      }
      const settled = await settleReceipt(
        receipt,
        context.label,
        context.baselineStatusRevision,
      );
      inFlightRef.current.delete(queryThreadId);
      return settled ? receipt : null;
    },
    [
      attempt,
      queryClient,
      queryThreadId,
      ready,
      scope,
      setAttempt,
      settleReceipt,
      statusKey,
      threadId,
    ],
  );

  const reconcile = React.useCallback(async () => {
    if (threadId === null || inFlightRef.current.has(queryThreadId)) return;
    inFlightRef.current.add(queryThreadId);
    const initialAttempt = attempt;
    const initialRevision = maximumRevision(
      initialAttempt?.baselineStatusRevision,
      status?.statusRevision,
    );
    const recoveryOperation =
      status?.state === "recoveryRequired" ? status.operation : null;
    const cachedBlockingReceipt =
      status?.state === "ready" ? status.blockingReceipt : null;
    const knownReceipt = initialAttempt?.receipt ?? cachedBlockingReceipt;
    const expectedRequestGeneration =
      initialAttempt?.requestGeneration ??
      cachedBlockingReceipt?.requestGeneration ??
      status?.writeGeneration ??
      null;
    let observedOperationId =
      initialAttempt?.operationId ??
      recoveryOperation?.operationId ??
      cachedBlockingReceipt?.operationId ??
      null;
    let observedOperation =
      initialAttempt?.operation ??
      recoveryOperation?.operation ??
      cachedBlockingReceipt?.operation ??
      null;
    try {
      await queryClient.cancelQueries({ exact: true, queryKey: statusKey });
      const result = await pollCodeGitReconcile({
        reconcile: () =>
          codeWorkspaceApi.reconcileCodeThreadGit({ scope, threadId }),
        wait: (milliseconds) =>
          new Promise((resolve) =>
            globalThis.setTimeout(resolve, milliseconds),
          ),
        onProgress: (progress) => {
          if (!reconcileResultMatches(progress, scope, threadId)) {
            throw new TypeError(
              "Native Git reconciliation did not match the selected thread",
            );
          }
          observedOperationId = progress.operationId;
          observedOperation = progress.operation;
          setAttempt({
            state: "reconciling",
            operation: progress.operation,
            operationId: progress.operationId,
            label: initialAttempt?.label ?? progress.operation,
            requestGeneration: expectedRequestGeneration,
            baselineStatusRevision: initialRevision,
            receipt: initialAttempt?.receipt ?? null,
            message: null,
          });
        },
      });
      if (
        result.state !== "exhausted" &&
        !reconcileResultMatches(result, scope, threadId)
      ) {
        throw new TypeError(
          "Native Git reconciliation did not match the selected thread",
        );
      }
      if (result.state === "exhausted") {
        setAttempt({
          state: "unknown",
          operation: result.operation,
          operationId: result.operationId,
          label: initialAttempt?.label ?? result.operation,
          requestGeneration: expectedRequestGeneration,
          baselineStatusRevision: initialRevision,
          receipt: initialAttempt?.receipt ?? null,
          message:
            "Native Git recovery is still pending after bounded checks. Check operation status again when ready.",
        });
      } else if (result.state === "completed") {
        const receiptError = codeGitReconcileReceiptError({
          expectedOperation: observedOperation,
          expectedOperationId: observedOperationId,
          expectedRequestGeneration,
          receipt: result.receipt,
        });
        if (receiptError !== null) throw new TypeError(receiptError);
        const minimumRevision = codeGitReceiptsMatch(
          cachedBlockingReceipt,
          result.receipt,
        )
          ? (initialAttempt?.baselineStatusRevision ?? null)
          : initialRevision;
        await settleReceipt(
          result.receipt,
          initialAttempt?.label ?? result.receipt.operation,
          minimumRevision,
        );
      } else if (result.state === "uncertain") {
        setAttempt({
          state: "uncertain",
          operation: result.operation,
          operationId: result.operationId,
          label: initialAttempt?.label ?? result.operation,
          requestGeneration: expectedRequestGeneration,
          baselineStatusRevision: initialRevision,
          receipt: null,
          message: result.message,
        });
      } else if (result.state === "none") {
        const refreshed = await readRawStatus();
        if (refreshed.state === "ready" && refreshed.blockingReceipt !== null) {
          const receiptError = codeGitReconcileReceiptError({
            expectedOperation: observedOperation,
            expectedOperationId: observedOperationId,
            expectedRequestGeneration,
            receipt: refreshed.blockingReceipt,
          });
          if (receiptError !== null) throw new TypeError(receiptError);
          await settleReceipt(
            refreshed.blockingReceipt,
            initialAttempt?.label ?? refreshed.blockingReceipt.operation,
            initialAttempt?.baselineStatusRevision ?? null,
          );
        } else if (refreshed.state === "recoveryRequired") {
          throw new TypeError(
            "Native Git recovery remains required after reconciliation",
          );
        } else {
          if (knownReceipt !== null) {
            if (
              refreshed.state !== "ready" ||
              refreshed.blockingReceipt !== null ||
              refreshed.writeGeneration !==
                knownReceipt.requestGeneration + 1 ||
              (initialRevision !== null &&
                refreshed.statusRevision <= initialRevision)
            ) {
              throw new TypeError(
                "Authoritative Git status did not prove the completed operation was cleared",
              );
            }
          } else if (
            expectedRequestGeneration !== null &&
            (refreshed.writeGeneration !== expectedRequestGeneration ||
              (initialRevision !== null &&
                refreshed.statusRevision < initialRevision))
          ) {
            throw new TypeError(
              "Authoritative Git status did not prove the unknown operation was absent",
            );
          }
          replaceStatus(refreshed);
          setAttempt(null);
        }
      }
    } catch (error) {
      const operation =
        observedOperation ?? initialAttempt?.operation ?? "commit";
      setAttempt({
        state: "unknown",
        operation,
        operationId: observedOperationId,
        label: initialAttempt?.label ?? operation,
        requestGeneration: expectedRequestGeneration,
        baselineStatusRevision: initialRevision,
        receipt: initialAttempt?.receipt ?? null,
        message: messageFrom(
          error,
          "Native Git reconciliation could not be verified.",
        ),
      });
    } finally {
      inFlightRef.current.delete(queryThreadId);
    }
  }, [
    attempt,
    queryClient,
    queryThreadId,
    readRawStatus,
    replaceStatus,
    scope,
    setAttempt,
    settleReceipt,
    status,
    statusKey,
    threadId,
  ]);

  const gitBlockerReason = blockerReason({
    attempt,
    enabled: queryEnabled,
    queryError: query.isError,
    queryPending: query.isPending,
    status,
  });
  const operationPending =
    attempt?.state === "pending" ||
    attempt?.state === "refreshing" ||
    attempt?.state === "reconciling";
  const commitPending =
    attempt?.operation === "commit" &&
    (attempt.state === "pending" || attempt.state === "refreshing");

  return {
    attempt,
    busy: gitBlockerReason !== null,
    commit,
    commitPending,
    gitBlockerReason,
    operationPending,
    query,
    ready,
    reconcile,
    retryStatus: query.refetch,
    runIndexMutation,
    status,
  };
}

export type CodeGitHandoffController = ReturnType<typeof useCodeGitHandoff>;
