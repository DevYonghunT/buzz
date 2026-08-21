import type {
  CodeGitMutationReceipt,
  CodeGitOperation,
  CodeGitReadyStatus,
  CodeGitReconcileResult,
  CodeGitStatus,
} from "../api/codeGitTypes";

export type CodeGitHandoffAttempt = {
  state: "pending" | "refreshing" | "reconciling" | "unknown" | "uncertain";
  operation: CodeGitOperation;
  operationId: string | null;
  label: string;
  requestGeneration: number | null;
  baselineStatusRevision: number | null;
  receipt: CodeGitMutationReceipt | null;
  message: string | null;
};

export const CODE_GIT_RECONCILE_DELAYS_MS = [250, 750, 1_500] as const;

type ReceiptSettlement =
  | {
      state: "settled";
      status: CodeGitReadyStatus;
      acknowledgementResponseLost: boolean;
    }
  | { state: "unknown"; message: string };

type ReconcilePollResult =
  | CodeGitReconcileResult
  | {
      state: "exhausted";
      operation: CodeGitOperation;
      operationId: string;
    };

function scopeMatches(
  left: CodeGitMutationReceipt["scope"],
  right: CodeGitMutationReceipt["scope"],
) {
  return (
    left.communityId === right.communityId &&
    left.projectDtag === right.projectDtag &&
    left.repositoryIdentity === right.repositoryIdentity
  );
}

export function codeGitReceiptsMatch(
  left: CodeGitMutationReceipt | null,
  right: CodeGitMutationReceipt,
): boolean {
  if (
    left === null ||
    left.operation !== right.operation ||
    left.operationId !== right.operationId ||
    left.threadId !== right.threadId ||
    left.requestGeneration !== right.requestGeneration ||
    left.beforeSnapshotId !== right.beforeSnapshotId ||
    left.disposition !== right.disposition ||
    !scopeMatches(left.scope, right.scope)
  ) {
    return false;
  }
  if (left.operation === "commit" && right.operation === "commit") {
    return (
      left.previousHead === right.previousHead &&
      left.commit === right.commit &&
      left.tree === right.tree
    );
  }
  return (
    left.operation !== "commit" &&
    right.operation !== "commit" &&
    left.fileId === right.fileId
  );
}

export function codeGitReconcileReceiptError({
  expectedOperation,
  expectedOperationId,
  expectedRequestGeneration,
  receipt,
}: {
  expectedOperation: CodeGitOperation | null;
  expectedOperationId: string | null;
  expectedRequestGeneration: number | null;
  receipt: CodeGitMutationReceipt;
}): string | null {
  if (
    expectedOperation === null ||
    expectedOperation !== receipt.operation ||
    (expectedOperationId !== null &&
      expectedOperationId !== receipt.operationId)
  ) {
    return "Completed Git receipt did not match the reconciliation coordinate";
  }
  if (
    expectedRequestGeneration === null ||
    expectedRequestGeneration !== receipt.requestGeneration
  ) {
    return "Completed Git receipt did not match the requested write generation";
  }
  return null;
}

function completedStatusError(
  status: CodeGitStatus,
  receipt: CodeGitMutationReceipt,
  minimumStatusRevision: number | null,
): string | null {
  if (
    status.state !== "ready" ||
    status.threadId !== receipt.threadId ||
    !scopeMatches(status.scope, receipt.scope)
  ) {
    return "Authoritative Git status did not match the completed operation.";
  }
  if (
    minimumStatusRevision !== null &&
    status.statusRevision <= minimumStatusRevision
  ) {
    return "Authoritative Git status did not advance after the completed operation.";
  }
  if (status.writeGeneration !== receipt.requestGeneration + 1) {
    return "Authoritative Git status did not confirm the receipt request generation.";
  }
  if (!codeGitReceiptsMatch(status.blockingReceipt, receipt)) {
    return "Authoritative Git status did not return the exact completed receipt.";
  }
  return null;
}

function clearedStatusError(
  status: CodeGitStatus,
  blocking: CodeGitReadyStatus,
): string | null {
  if (
    status.state !== "ready" ||
    status.threadId !== blocking.threadId ||
    !scopeMatches(status.scope, blocking.scope)
  ) {
    return "Authoritative Git status did not match the acknowledged operation.";
  }
  if (
    status.statusRevision <= blocking.statusRevision ||
    status.writeGeneration !== blocking.writeGeneration ||
    status.blockingReceipt !== null
  ) {
    return "Authoritative Git status did not confirm acknowledgement.";
  }
  return null;
}

function messageFrom(error: unknown, fallback: string): string {
  return error instanceof Error && error.message.trim()
    ? error.message
    : fallback;
}

export async function settleCodeGitReceipt({
  acceptStatus,
  acknowledge,
  minimumStatusRevision,
  readStatus,
  receipt,
}: {
  acceptStatus: (status: CodeGitReadyStatus) => void;
  acknowledge: (status: CodeGitReadyStatus) => Promise<void>;
  minimumStatusRevision: number | null;
  readStatus: () => Promise<CodeGitStatus>;
  receipt: CodeGitMutationReceipt;
}): Promise<ReceiptSettlement> {
  let blocking: CodeGitStatus;
  try {
    blocking = await readStatus();
  } catch (error) {
    return {
      state: "unknown",
      message: messageFrom(
        error,
        "Git write completed, but authoritative status could not be read.",
      ),
    };
  }
  const confirmationError = completedStatusError(
    blocking,
    receipt,
    minimumStatusRevision,
  );
  if (confirmationError !== null || blocking.state !== "ready") {
    return {
      state: "unknown",
      message: confirmationError ?? "Completed Git status was not ready.",
    };
  }
  try {
    acceptStatus(blocking);
  } catch (error) {
    return {
      state: "unknown",
      message: messageFrom(
        error,
        "Completed Git status could not be accepted as authoritative.",
      ),
    };
  }

  try {
    await acknowledge(blocking);
  } catch (acknowledgementError) {
    let afterLoss: CodeGitStatus;
    try {
      afterLoss = await readStatus();
    } catch (statusError) {
      return {
        state: "unknown",
        message: `${messageFrom(
          acknowledgementError,
          "Git acknowledgement response was lost.",
        )} ${messageFrom(
          statusError,
          "The cleared status could not be verified.",
        )}`,
      };
    }
    if (clearedStatusError(afterLoss, blocking) === null) {
      try {
        acceptStatus(afterLoss as CodeGitReadyStatus);
      } catch (error) {
        return {
          state: "unknown",
          message: messageFrom(
            error,
            "Cleared Git status could not be accepted as authoritative.",
          ),
        };
      }
      return {
        state: "settled",
        status: afterLoss as CodeGitReadyStatus,
        acknowledgementResponseLost: true,
      };
    }
    return {
      state: "unknown",
      message: codeGitReceiptsMatch(
        afterLoss.state === "ready" ? afterLoss.blockingReceipt : null,
        receipt,
      )
        ? "The acknowledgement response was lost and the completed Git operation is still blocking writes. Check operation status again."
        : "The acknowledgement response was lost and authoritative Git status did not prove whether the blocker cleared.",
    };
  }

  let cleared: CodeGitStatus;
  try {
    cleared = await readStatus();
  } catch (error) {
    return {
      state: "unknown",
      message: messageFrom(
        error,
        "Git acknowledgement completed, but cleared status could not be read.",
      ),
    };
  }
  const clearError = clearedStatusError(cleared, blocking);
  if (clearError !== null || cleared.state !== "ready") {
    return {
      state: "unknown",
      message: clearError ?? "Acknowledged Git status was not ready.",
    };
  }
  try {
    acceptStatus(cleared);
  } catch (error) {
    return {
      state: "unknown",
      message: messageFrom(
        error,
        "Cleared Git status could not be accepted as authoritative.",
      ),
    };
  }
  return {
    state: "settled",
    status: cleared,
    acknowledgementResponseLost: false,
  };
}

export async function pollCodeGitReconcile({
  onProgress,
  reconcile,
  wait,
}: {
  onProgress: (
    result: Extract<
      CodeGitReconcileResult,
      { state: "pending" | "recovering" }
    >,
  ) => void;
  reconcile: () => Promise<CodeGitReconcileResult>;
  wait: (milliseconds: number) => Promise<void>;
}): Promise<ReconcilePollResult> {
  let coordinate: { operation: CodeGitOperation; operationId: string } | null =
    null;
  for (let poll = 0; ; poll += 1) {
    const result = await reconcile();
    if (result.state !== "pending" && result.state !== "recovering") {
      return result;
    }
    if (
      coordinate !== null &&
      (coordinate.operationId !== result.operationId ||
        coordinate.operation !== result.operation)
    ) {
      throw new TypeError("Native Git reconciliation coordinate changed");
    }
    coordinate = {
      operation: result.operation,
      operationId: result.operationId,
    };
    onProgress(result);
    const delay = CODE_GIT_RECONCILE_DELAYS_MS[poll];
    if (delay === undefined) {
      return { state: "exhausted", ...coordinate };
    }
    await wait(delay);
  }
}
