import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, LoaderCircle, RefreshCw, Trash2 } from "lucide-react";
import * as React from "react";

import { codeWorkspaceApi } from "../api/codeWorkspace";
import type {
  CodeThreadBindingScope,
  CodeWorktreeInventoryBlocker,
  CodeWorktreeInventoryRow,
  CodeWorktreeRemovalReceipt,
} from "../api/types";
import {
  codeSessionQueryKeys,
  codeWorktreesQueryOptions,
} from "../state/codeSessionQueries";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";

const CODE_WORKTREE_BLOCKER_LABELS = {
  activeBinding: "Active task",
  lifecycleUnsettled: "Task lifecycle is unsettled",
  unfinishedPreparation: "Unfinished task preparation",
  localCheckout: "Local checkout",
  unavailableRoot: "Worktree unavailable",
  dirtyRoot: "Uncommitted changes",
  branchAttached: "Branch is attached",
  headDrift: "HEAD differs from its immutable base",
  mergeProofUnavailable: "Merge proof unavailable",
} as const satisfies Record<CodeWorktreeInventoryBlocker, string>;

export function codeWorktreeBlockerLabel(
  blocker: CodeWorktreeInventoryBlocker,
): string {
  return CODE_WORKTREE_BLOCKER_LABELS[blocker];
}

function inventoryErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function authorityLabel(row: CodeWorktreeInventoryRow): string {
  if (row.authority.type === "binding") {
    return `Task · ${row.authority.lifecycle}`;
  }
  return `${row.authority.operation === "fork" ? "Fork" : "Task"} preparation · ${row.authority.state}`;
}

type CodeWorktreeRemovalAttempt = {
  readonly row: CodeWorktreeInventoryRow;
  readonly threadId: string;
  readonly receipt: CodeWorktreeRemovalReceipt | null;
};

function CodeWorktreeInventoryRowView({
  actionsReady,
  onRemove,
  removalAttempted,
  removalBlocked,
  removalCommitted,
  removalPending,
  row,
}: {
  actionsReady: boolean;
  onRemove: (row: CodeWorktreeInventoryRow, trigger: HTMLButtonElement) => void;
  removalAttempted: boolean;
  removalBlocked: boolean;
  removalCommitted: boolean;
  removalPending: boolean;
  row: CodeWorktreeInventoryRow;
}) {
  const removableThreadId =
    row.canRemove &&
    row.authority.type === "binding" &&
    row.authority.lifecycle === "archived"
      ? row.authority.threadId
      : null;
  const removalActionLabel = removableThreadId
    ? removalCommitted
      ? `Refresh task lists for removed task ${removableThreadId}`
      : removalAttempted
        ? `Retry exact worktree removal for task ${removableThreadId}`
        : `Remove worktree for task ${removableThreadId}`
    : undefined;
  return (
    <li
      className="rounded-lg border border-border/70 bg-background/60 p-2.5"
      data-testid={`code-worktree-${row.descriptor.worktreeId}`}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <p className="truncate text-xs font-medium text-foreground">
            Managed worktree
          </p>
          <p className="mt-0.5 truncate text-2xs text-muted-foreground">
            {authorityLabel(row)}
          </p>
        </div>
        <span className="shrink-0 rounded-md border border-border/70 px-1.5 py-0.5 text-2xs font-medium text-muted-foreground">
          {removalCommitted
            ? "Removal completed"
            : removalPending
              ? "Removal in progress"
              : removalAttempted
                ? "Outcome pending"
                : removableThreadId
                  ? "Ready to remove"
                  : "Preserved"}
        </span>
      </div>
      <p
        className="mt-1.5 truncate font-mono text-2xs text-muted-foreground"
        title={row.descriptor.executionRoot}
      >
        {row.descriptor.executionRoot}
      </p>
      {row.inspection.status === "unavailable" ? (
        <p
          className="mt-2 text-pretty text-2xs text-destructive"
          data-testid="code-worktree-unavailable-error"
        >
          {row.inspection.error}
        </p>
      ) : null}
      {row.blockers.length > 0 ? (
        <ul
          aria-label="Preservation blockers"
          className="mt-2 space-y-1 text-2xs text-muted-foreground"
        >
          {row.blockers.map((blocker) => (
            <li className="flex items-start gap-1.5" key={blocker}>
              <span
                aria-hidden="true"
                className="mt-1 size-1 rounded-full bg-current"
              />
              <span className="text-pretty">
                {codeWorktreeBlockerLabel(blocker)}
              </span>
            </li>
          ))}
        </ul>
      ) : null}
      {removableThreadId ? (
        <Button
          aria-label={removalActionLabel}
          className="mt-2 w-full text-destructive hover:text-destructive"
          data-testid={`code-worktree-remove-${removableThreadId}`}
          disabled={!actionsReady || removalBlocked || removalPending}
          onClick={(event) => onRemove(row, event.currentTarget)}
          size="xs"
          type="button"
          variant="outline"
        >
          {removalPending ? (
            <LoaderCircle className="animate-spin motion-reduce:animate-none" />
          ) : removalCommitted || removalAttempted ? (
            <RefreshCw />
          ) : (
            <Trash2 />
          )}
          {removalPending
            ? removalCommitted
              ? "Refreshing…"
              : "Confirming…"
            : removalCommitted
              ? "Refresh task lists"
              : removalAttempted
                ? "Retry removal"
                : "Remove worktree"}
        </Button>
      ) : null}
    </li>
  );
}

/** Query-owning projection of native-authorized managed roots. */
export function CodeWorktreeInventorySection({
  actionsReady,
  scope,
}: {
  actionsReady: boolean;
  scope: CodeThreadBindingScope;
}) {
  const queryClient = useQueryClient();
  const inventoryQuery = useQuery(codeWorktreesQueryOptions(scope));
  const removalAttemptQuery = useQuery<CodeWorktreeRemovalAttempt | null>({
    enabled: false,
    gcTime: Number.POSITIVE_INFINITY,
    initialData: null,
    queryFn: async () => null,
    queryKey: codeSessionQueryKeys.worktreeRemovalAttempt(scope),
    staleTime: Number.POSITIVE_INFINITY,
  });
  const rows = inventoryQuery.data ?? [];
  const inventoryRefreshButtonRef = React.useRef<HTMLButtonElement>(null);
  const removalTriggerRef = React.useRef<HTMLButtonElement | null>(null);
  const [removalRow, setRemovalRow] =
    React.useState<CodeWorktreeInventoryRow | null>(null);
  const removalAttempt = removalAttemptQuery.data;
  const committedReceipt = removalAttempt?.receipt ?? null;
  const setRemovalAttempt = React.useCallback(
    (attempt: CodeWorktreeRemovalAttempt | null) => {
      queryClient.setQueryData(
        codeSessionQueryKeys.worktreeRemovalAttempt(scope),
        attempt,
      );
    },
    [queryClient, scope],
  );
  const [reconciliationError, setReconciliationError] =
    React.useState<unknown>(null);
  const [reconciliationPending, setReconciliationPending] =
    React.useState(false);
  const [lastRemovalAnnouncement, setLastRemovalAnnouncement] =
    React.useState("");
  const removalSubmissionRef = React.useRef(false);
  const removalMutation = useMutation({
    mutationFn: async (threadId: string) => {
      const receipt = await codeWorkspaceApi.removeCodeWorktree({
        scope,
        threadId,
      });
      return receipt;
    },
  });
  const reconcileRemoval = React.useCallback(
    async (
      receipt: CodeWorktreeRemovalReceipt,
      focusFallbackFrom?: HTMLButtonElement,
    ) => {
      setReconciliationPending(true);
      setReconciliationError(null);
      try {
        await Promise.all([
          queryClient.cancelQueries({
            exact: true,
            queryKey: codeSessionQueryKeys.worktrees(scope),
          }),
          queryClient.cancelQueries({
            exact: true,
            queryKey: codeSessionQueryKeys.threads(scope),
          }),
        ]);
        // Bypass QueryClient request de-duplication so these reads are known to
        // begin after the irreversible native command returned its receipt.
        // Cancelling first also prevents a pre-removal query from committing
        // stale data after these fresh snapshots.
        const [refreshedRows, refreshedThreads] = await Promise.all([
          codeWorkspaceApi.listCodeWorktrees({ scope }),
          codeWorkspaceApi.listCodeThreads({ scope }),
        ]);
        if (
          refreshedRows.some(
            (candidate) =>
              candidate.authority.type === "binding" &&
              candidate.authority.threadId === receipt.threadId,
          ) ||
          refreshedThreads.data.some(
            (candidate) => candidate.binding.codexThreadId === receipt.threadId,
          )
        ) {
          throw new Error(
            "The worktree was removed, but its authoritative task lists did not reconcile.",
          );
        }
        queryClient.setQueryData(
          codeSessionQueryKeys.worktrees(scope),
          refreshedRows,
        );
        queryClient.setQueryData(
          codeSessionQueryKeys.threads(scope),
          refreshedThreads,
        );
        setLastRemovalAnnouncement(
          `Worktree removed for task ${receipt.threadId}. Its Code task transcript was preserved.`,
        );
        const shouldReturnFocus =
          focusFallbackFrom?.isConnected === true &&
          focusFallbackFrom.ownerDocument.activeElement === focusFallbackFrom;
        setRemovalRow(null);
        setRemovalAttempt(null);
        if (shouldReturnFocus) {
          globalThis.requestAnimationFrame(() => {
            inventoryRefreshButtonRef.current?.focus();
          });
        }
      } catch (error) {
        setReconciliationError(error);
      } finally {
        setReconciliationPending(false);
      }
    },
    [queryClient, scope, setRemovalAttempt],
  );
  const removalPending = removalMutation.isPending || reconciliationPending;

  const openRemovalDialog = React.useCallback(
    (row: CodeWorktreeInventoryRow, trigger: HTMLButtonElement) => {
      const rowThreadId =
        row.authority.type === "binding" ? row.authority.threadId : null;
      if (removalAttempt !== null && removalAttempt.threadId !== rowThreadId) {
        return;
      }
      removalTriggerRef.current = trigger;
      removalMutation.reset();
      if (committedReceipt === null) {
        setReconciliationError(null);
      }
      setRemovalRow(row);
    },
    [committedReceipt, removalAttempt, removalMutation.reset],
  );

  const resumeRemovalAttempt = React.useCallback(
    async (
      attempt: CodeWorktreeRemovalAttempt,
      focusFallbackFrom?: HTMLButtonElement,
    ) => {
      if (removalPending || !actionsReady || removalSubmissionRef.current) {
        return;
      }
      removalSubmissionRef.current = true;
      try {
        if (attempt.receipt !== null) {
          await reconcileRemoval(attempt.receipt, focusFallbackFrom);
          return;
        }
        const receipt = await removalMutation.mutateAsync(attempt.threadId);
        setRemovalAttempt({ ...attempt, receipt });
        await reconcileRemoval(receipt, focusFallbackFrom);
      } catch {
        // Keep the exact attempt coordinate when the native outcome may have
        // committed before its response was lost. The mutation error remains
        // visible, and the same request can recover the durable receipt.
      } finally {
        removalSubmissionRef.current = false;
      }
    },
    [
      actionsReady,
      reconcileRemoval,
      removalMutation.mutateAsync,
      removalPending,
      setRemovalAttempt,
    ],
  );

  const confirmRemoval = React.useCallback(async () => {
    if (
      removalRow === null ||
      removalPending ||
      !actionsReady ||
      removalSubmissionRef.current
    )
      return;
    const rowThreadId =
      removalRow.authority.type === "binding" && removalRow.canRemove
        ? removalRow.authority.threadId
        : null;
    if (rowThreadId === null) {
      return;
    }
    if (removalAttempt !== null && removalAttempt.threadId !== rowThreadId) {
      return;
    }
    const attempt = removalAttempt ?? {
      row: removalRow,
      threadId: rowThreadId,
      receipt: null,
    };
    if (removalAttempt === null) {
      // Persist the user-confirmed coordinate before crossing the native
      // bridge. A lost response can then be retried after this UI unmounts.
      setRemovalAttempt(attempt);
    }
    await resumeRemovalAttempt(attempt);
  }, [
    actionsReady,
    removalAttempt,
    removalPending,
    removalRow,
    resumeRemovalAttempt,
    setRemovalAttempt,
  ]);

  return (
    <section
      aria-labelledby="code-worktree-inventory-heading"
      className="mt-3 border-border/60 border-t pt-3"
      data-testid="code-worktree-inventory"
    >
      <div className="flex items-center gap-2 px-1">
        <h3
          className="min-w-0 flex-1 text-balance text-xs font-semibold text-foreground"
          id="code-worktree-inventory-heading"
        >
          Managed worktrees
        </h3>
        <Button
          aria-busy={removalPending || undefined}
          aria-label={
            removalAttempt
              ? committedReceipt
                ? "Refresh authoritative task lists after worktree removal"
                : "Review exact worktree removal retry"
              : "Refresh managed worktrees"
          }
          disabled={
            inventoryQuery.isFetching ||
            (removalAttempt !== null && !actionsReady)
          }
          onClick={(event) => {
            if (removalAttempt) {
              if (committedReceipt) {
                void resumeRemovalAttempt(removalAttempt);
              } else {
                openRemovalDialog(removalAttempt.row, event.currentTarget);
              }
            } else {
              void inventoryQuery.refetch();
            }
          }}
          ref={inventoryRefreshButtonRef}
          size="icon-xs"
          title={
            removalAttempt
              ? committedReceipt
                ? "Refresh authoritative task lists after worktree removal"
                : "Review exact worktree removal retry"
              : "Refresh managed worktrees"
          }
          variant="ghost"
        >
          <RefreshCw
            className={
              removalPending
                ? "animate-spin motion-reduce:animate-none"
                : undefined
            }
          />
        </Button>
      </div>

      {removalAttempt !== null && removalRow === null ? (
        <div
          aria-live={
            reconciliationError || removalMutation.error ? undefined : "polite"
          }
          className={
            reconciliationError || removalMutation.error
              ? "mt-2 rounded-lg border border-destructive/30 bg-destructive/10 p-2.5"
              : "mt-2 rounded-lg border border-border/70 bg-muted/30 p-2.5"
          }
          data-testid="code-worktree-removal-reconciliation"
          role={
            reconciliationError || removalMutation.error ? "alert" : "status"
          }
        >
          <p
            className={`text-pretty text-2xs ${
              reconciliationError || removalMutation.error
                ? "text-destructive"
                : "text-muted-foreground"
            }`}
          >
            {committedReceipt
              ? `Worktree removal completed for task ${committedReceipt.threadId}. The Code task transcript is preserved. Refresh the authoritative task lists before removing another worktree.`
              : `The removal outcome for task ${removalAttempt.threadId} is unknown. Retrying the exact native request may complete permanent removal or return its durable receipt. This operation never removes the Code task transcript.`}
            {reconciliationError
              ? ` ${inventoryErrorMessage(reconciliationError)}`
              : removalMutation.error
                ? ` ${inventoryErrorMessage(removalMutation.error)}`
                : ""}
          </p>
          <Button
            aria-busy={removalPending || undefined}
            className="mt-2"
            disabled={!actionsReady}
            onClick={(event) => {
              if (committedReceipt) {
                void resumeRemovalAttempt(removalAttempt, event.currentTarget);
              } else {
                openRemovalDialog(removalAttempt.row, event.currentTarget);
              }
            }}
            size="xs"
            variant={committedReceipt ? "outline" : "destructive"}
          >
            {committedReceipt
              ? "Retry task-list refresh"
              : "Review exact removal retry"}
          </Button>
        </div>
      ) : null}

      {inventoryQuery.isPending ? (
        <div
          aria-label="Loading managed worktrees"
          className="space-y-2 px-1 pt-2"
          role="status"
        >
          <Skeleton className="h-16" pulsing={false} />
          <Skeleton className="h-16" pulsing={false} />
        </div>
      ) : inventoryQuery.error ? (
        <div
          className="mt-2 rounded-lg border border-destructive/30 bg-destructive/10 p-2.5"
          role="alert"
        >
          <div className="flex items-start gap-2 text-2xs text-destructive">
            <AlertTriangle className="mt-0.5 size-3.5 shrink-0" />
            <p className="min-w-0 text-pretty">
              {inventoryErrorMessage(inventoryQuery.error)}
            </p>
          </div>
          <Button
            className="mt-2"
            onClick={() => void inventoryQuery.refetch()}
            size="xs"
            variant="outline"
          >
            Retry inventory
          </Button>
        </div>
      ) : rows.length === 0 ? (
        <p className="px-1 py-3 text-pretty text-2xs text-muted-foreground">
          Create a managed task to see its preserved worktree here.
        </p>
      ) : (
        <ul aria-label="Managed worktrees" className="mt-2 space-y-2">
          {rows.map((row) => (
            <CodeWorktreeInventoryRowView
              actionsReady={actionsReady}
              key={`${row.authority.type}:${
                row.authority.type === "binding"
                  ? row.authority.threadId
                  : row.authority.preparationId
              }`}
              onRemove={openRemovalDialog}
              removalAttempted={
                removalAttempt !== null &&
                committedReceipt === null &&
                row.authority.type === "binding" &&
                row.authority.threadId === removalAttempt.threadId
              }
              removalBlocked={
                removalAttempt !== null &&
                (row.authority.type !== "binding" ||
                  row.authority.threadId !== removalAttempt.threadId)
              }
              removalCommitted={
                committedReceipt !== null &&
                row.authority.type === "binding" &&
                row.authority.threadId === committedReceipt.threadId
              }
              removalPending={
                removalPending &&
                removalRow?.authority.type === "binding" &&
                row.authority.type === "binding" &&
                removalRow.authority.threadId === row.authority.threadId
              }
              row={row}
            />
          ))}
        </ul>
      )}
      <AlertDialog
        onOpenChange={(open) => {
          if (!open && !removalPending) {
            setRemovalRow(null);
            removalMutation.reset();
            if (committedReceipt === null) {
              setReconciliationError(null);
            }
          }
        }}
        open={removalRow !== null}
      >
        <AlertDialogContent
          data-testid="code-worktree-remove-dialog"
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            const trigger = removalTriggerRef.current;
            const focusTarget =
              trigger?.isConnected === true
                ? trigger
                : inventoryRefreshButtonRef.current;
            focusTarget?.focus();
            removalTriggerRef.current = null;
          }}
        >
          <AlertDialogHeader>
            <AlertDialogTitle className="text-balance">
              {committedReceipt
                ? `Worktree removal completed for task ${committedReceipt.threadId}`
                : removalAttempt && removalMutation.isPending
                  ? `Removing worktree for task ${removalAttempt.threadId}`
                  : removalAttempt
                    ? `Retry exact worktree removal for task ${removalAttempt.threadId}?`
                    : removalRow?.authority.type === "binding"
                      ? `Remove worktree for task ${removalRow.authority.threadId}?`
                      : "Remove this worktree?"}
            </AlertDialogTitle>
            <AlertDialogDescription className="text-pretty">
              {committedReceipt
                ? "The execution root is removed and the Code task transcript is preserved. Refresh the authoritative task lists to finish reconciling this view."
                : removalAttempt && removalMutation.isPending
                  ? "The exact native removal request is in progress. The Code task transcript is never removed."
                  : removalAttempt
                    ? "The previous attempt did not return a durable receipt. Retrying this exact request may complete permanent removal or return the existing receipt without selecting a new execution root. The Code task transcript is preserved, but the execution root cannot be restored if removal completes."
                    : "This permanently removes the clean managed worktree after native merge verification. The Code task transcript is preserved, but this execution root cannot be restored."}
              {removalRow ? (
                <span className="mt-2 block break-all font-mono text-2xs text-foreground">
                  {removalRow.descriptor.executionRoot}
                </span>
              ) : null}
            </AlertDialogDescription>
          </AlertDialogHeader>
          {reconciliationError ? (
            <p className="text-pretty text-sm text-destructive" role="alert">
              Worktree removal completed, but the authoritative task lists could
              not be refreshed. {inventoryErrorMessage(reconciliationError)}
            </p>
          ) : removalMutation.error ? (
            <p className="text-pretty text-sm text-destructive" role="alert">
              {inventoryErrorMessage(removalMutation.error)}
            </p>
          ) : null}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={removalPending}>
              {removalAttempt ? "Close" : "Cancel"}
            </AlertDialogCancel>
            <AlertDialogAction
              className={
                committedReceipt
                  ? undefined
                  : "bg-destructive text-destructive-foreground hover:bg-destructive/90"
              }
              disabled={!actionsReady || removalPending}
              onClick={(event) => {
                event.preventDefault();
                void confirmRemoval();
              }}
            >
              {removalPending ? (
                <LoaderCircle className="animate-spin motion-reduce:animate-none" />
              ) : committedReceipt ? (
                <RefreshCw />
              ) : (
                <Trash2 />
              )}
              {removalPending
                ? committedReceipt
                  ? "Refreshing…"
                  : "Confirming…"
                : committedReceipt
                  ? "Refresh task lists"
                  : removalAttempt
                    ? "Retry exact removal"
                    : "Remove worktree"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <p aria-live="polite" className="sr-only" role="status">
        {lastRemovalAnnouncement}
      </p>
    </section>
  );
}
