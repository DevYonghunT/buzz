import { useQuery } from "@tanstack/react-query";
import { Files, RefreshCw, X } from "lucide-react";
import * as React from "react";

import { ProjectDiffFilesPanel } from "@/features/projects/ui/ProjectPullRequestFilesChangedPanel";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import type { CodeGitChangeSet } from "../api/codeGitTypes";
import type { CodeThreadBinding, CodeThreadBindingScope } from "../api/types";
import { codeThreadChangesQueryOptions } from "../state/codeSessionQueries";
import type { CodeGitHandoffController } from "../state/useCodeGitHandoff";
import { CodeCommitDialog } from "./CodeCommitDialog";
import { CodeGitChangesActions } from "./CodeGitChangesActions";

export function CodeChangesPanel({
  binding,
  className,
  controller: git,
  enabled,
  onClose,
  runtimeGeneration,
  scope,
}: {
  binding: CodeThreadBinding;
  className?: string;
  controller: CodeGitHandoffController;
  enabled: boolean;
  onClose: () => void;
  runtimeGeneration: number;
  scope: CodeThreadBindingScope;
}) {
  const [commitOpen, setCommitOpen] = React.useState(false);
  const useLegacy =
    binding.executionMode !== "worktree" ||
    git.query.isError ||
    git.status?.state === "blocked" ||
    git.status?.state === "recoveryRequired";
  const changesQuery = useQuery({
    ...codeThreadChangesQueryOptions({
      scope,
      threadId: binding.codexThreadId,
      runtimeGeneration,
    }),
    enabled: enabled && useLegacy,
    refetchOnMount: "always",
    staleTime: 1_000,
  });
  const changes = git.ready ? toLegacyDiff(git.ready.task) : changesQuery.data;
  const truncatedPatchCount =
    changes?.files.filter((file) => file.truncated).length ?? 0;
  const refreshChanges = () => {
    if (!enabled || changesQuery.isFetching || git.query.isFetching) return;
    if (useLegacy) void changesQuery.refetch();
    else void git.query.refetch();
  };

  return (
    <aside
      aria-label="Changes inspector"
      className={cn(
        "flex min-h-0 flex-col border-border/60 border-l bg-background",
        className,
      )}
      data-testid="code-changes-inspector"
      onKeyDown={(event) => {
        if (event.key === "Escape") onClose();
      }}
    >
      <header className="flex h-11 shrink-0 items-center gap-2 border-border/60 border-b px-3">
        <Files className="h-3.5 w-3.5 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <h2 className="text-balance text-sm font-semibold" tabIndex={-1}>
            Changes
          </h2>
          <p className="truncate text-2xs text-muted-foreground">
            {binding.executionMode === "worktree"
              ? "Managed worktree"
              : "Local checkout"}
            {` · base ${binding.baseRef.slice(0, 8)}`}
          </p>
        </div>
        <Button
          aria-label="Refresh changed files"
          className="h-7 w-7"
          disabled={
            !enabled ||
            changesQuery.isFetching ||
            git.query.isFetching ||
            git.busy
          }
          onClick={refreshChanges}
          size="icon-xs"
          title="Refresh changed files"
          variant="ghost"
        >
          <RefreshCw
            className={cn(
              (changesQuery.isFetching || git.query.isFetching) &&
                "animate-spin motion-reduce:animate-none",
            )}
          />
        </Button>
        <Button
          aria-label="Close Changes inspector"
          className="h-7 w-7"
          onClick={onClose}
          size="icon-xs"
          title="Close Changes inspector"
          variant="ghost"
        >
          <X />
        </Button>
      </header>
      <div className="min-h-0 flex-1 overflow-auto">
        {!enabled ? (
          <div
            className="p-4 text-sm text-muted-foreground"
            data-testid="code-changes-sync-pending"
          >
            Waiting for Code activity synchronization…
          </div>
        ) : (
          <>
            {git.attempt ? (
              <div
                className={cn(
                  "border-b px-4 py-3 text-pretty text-xs",
                  git.attempt.state === "unknown" ||
                    git.attempt.state === "uncertain"
                    ? "border-destructive/30 bg-destructive/10 text-destructive"
                    : "border-border bg-muted/40 text-muted-foreground",
                )}
                role={
                  git.attempt.state === "unknown" ||
                  git.attempt.state === "uncertain"
                    ? "alert"
                    : "status"
                }
              >
                <p>
                  {git.attempt.state === "pending"
                    ? `Applying ${git.attempt.label}…`
                    : git.attempt.state === "refreshing"
                      ? "Git write completed. Confirming authoritative status…"
                      : git.attempt.state === "reconciling"
                        ? "Checking native Git operation status…"
                        : git.attempt.message}
                </p>
                {git.attempt.state === "unknown" ||
                git.attempt.state === "uncertain" ? (
                  <Button
                    className="mt-2"
                    onClick={() => void git.reconcile()}
                    size="sm"
                    variant="outline"
                  >
                    Check operation status
                  </Button>
                ) : null}
              </div>
            ) : null}
            {git.query.isError && git.attempt === null ? (
              <div
                className="border-destructive/30 border-b bg-destructive/10 px-4 py-3 text-pretty text-xs text-destructive"
                role="alert"
              >
                Git write status could not be verified. Task diff remains
                read-only. {String(git.query.error)}
                <Button
                  className="mt-2"
                  onClick={() => void git.retryStatus()}
                  size="sm"
                  variant="outline"
                >
                  Retry Git status
                </Button>
              </div>
            ) : null}
            {git.status?.state === "blocked" ? (
              <div
                className="border-border border-b bg-muted/40 px-4 py-3 text-pretty text-xs"
                role="status"
              >
                <p>{git.status.reason}</p>
                <p className="text-muted-foreground">
                  {git.status.remediation}
                </p>
              </div>
            ) : null}
            {git.status?.state === "recoveryRequired" ? (
              <div
                className="border-amber-500/20 border-b bg-amber-500/10 px-4 py-3 text-pretty text-xs text-amber-700 dark:text-amber-300"
                role="alert"
              >
                A {git.status.operation.operation} operation requires
                reconciliation.
                <Button
                  className="mt-2"
                  onClick={() => void git.reconcile()}
                  size="sm"
                  variant="outline"
                >
                  Check operation status
                </Button>
              </div>
            ) : null}
            {git.ready !== null &&
            git.ready.blockingReceipt !== null &&
            git.attempt === null ? (
              <div
                className="border-amber-500/20 border-b bg-amber-500/10 px-4 py-3 text-pretty text-xs text-amber-700 dark:text-amber-300"
                role="alert"
              >
                The completed {git.ready.blockingReceipt.operation} operation
                must be acknowledged before Git writes or Code turns can
                continue.
                <Button
                  className="mt-2"
                  onClick={() => void git.reconcile()}
                  size="sm"
                  variant="outline"
                >
                  Complete Git handoff
                </Button>
              </div>
            ) : null}
            {git.ready ? (
              <>
                <CodeGitChangesActions
                  busy={git.busy}
                  onCommit={() => setCommitOpen(true)}
                  onMutate={(operation, file) =>
                    void git.runIndexMutation(operation, file)
                  }
                  status={git.ready}
                />
                <CodeCommitDialog
                  blockedReason={git.gitBlockerReason}
                  onCommit={git.commit}
                  onOpenChange={setCommitOpen}
                  open={commitOpen}
                  status={git.ready}
                  submitting={git.commitPending}
                />
                <div className="border-border/60 border-t px-4 pt-4">
                  <h3 className="text-balance text-sm font-semibold">
                    Task diff
                  </h3>
                  <p className="text-pretty text-xs text-muted-foreground">
                    Persisted base to working tree, retained after local
                    commits.
                  </p>
                </div>
              </>
            ) : null}
            {changes && (changes.filesTruncated || truncatedPatchCount > 0) ? (
              <div
                className="space-y-1 border-amber-500/20 border-b bg-amber-500/10 px-4 py-3 text-xs text-amber-700 dark:text-amber-300"
                data-testid="code-changes-completeness-warning"
                role="status"
              >
                {changes.filesTruncated ? (
                  <p>
                    Showing {changes.files.length} of {changes.totalFiles}{" "}
                    changed files. Review the local checkout for the complete
                    file list. Addition and deletion totals cover the shown
                    files only.
                  </p>
                ) : null}
                {truncatedPatchCount > 0 ? (
                  <p>
                    {changes.filesTruncated ? "Among the shown files, " : ""}
                    {truncatedPatchCount} file patch
                    {truncatedPatchCount === 1 ? "" : "es"} truncated. Review
                    the local checkout for the complete diff.
                  </p>
                ) : null}
              </div>
            ) : null}
            <ProjectDiffFilesPanel
              diff={changes}
              embedded
              error={useLegacy ? changesQuery.error : null}
              headerLabel={`Base ${binding.baseRef.slice(0, 8)}`}
              isLoading={
                changes !== undefined
                  ? false
                  : useLegacy
                    ? changesQuery.isLoading
                    : git.query.isLoading
              }
              subjectLabel="code task"
            />
          </>
        )}
      </div>
    </aside>
  );
}

function toLegacyDiff(changeSet: CodeGitChangeSet) {
  return {
    ...changeSet,
    files: changeSet.files.map(({ fileId: _fileId, ...file }) => file),
    commitBody: null,
  };
}
