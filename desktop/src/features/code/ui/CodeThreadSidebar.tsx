import {
  CircleDashed,
  GitBranch,
  LoaderCircle,
  Plus,
  RefreshCw,
  RotateCcw,
} from "lucide-react";

import type {
  CodeBoundThreadSummary,
  CodeThreadPreparation,
} from "../api/types";
import {
  codeThreadLabel,
  groupCodePreparations,
} from "../lib/codeWorkspaceView";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

function PreparationRow({
  actionPending,
  onOpen,
  preparation,
}: {
  actionPending: boolean;
  onOpen: (preparation: CodeThreadPreparation) => void;
  preparation: CodeThreadPreparation;
}) {
  const recovering = preparation.state === "starting";
  return (
    <div
      className="rounded-lg border border-amber-500/25 bg-amber-500/5 p-2.5"
      data-testid={`code-preparation-${preparation.preparationId}`}
    >
      <div className="flex items-start gap-2">
        {recovering ? (
          <RotateCcw className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
        ) : (
          <CircleDashed className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
        )}
        <div className="min-w-0 flex-1">
          <p className="text-xs font-medium text-foreground">
            {recovering ? "Needs recovery" : "Prepared task"}
          </p>
          <p className="mt-0.5 truncate font-mono text-2xs text-muted-foreground">
            {preparation.executionRoot}
          </p>
        </div>
      </div>
      <Button
        className="mt-2 w-full"
        disabled={actionPending}
        onClick={() => onOpen(preparation)}
        size="xs"
        variant="outline"
      >
        {actionPending ? (
          <LoaderCircle className="animate-spin motion-reduce:animate-none" />
        ) : null}
        {recovering ? "Recover task" : "Start task"}
      </Button>
    </div>
  );
}

function ThreadRow({
  onSelect,
  selected,
  thread,
}: {
  onSelect: (threadId: string) => void;
  selected: boolean;
  thread: CodeBoundThreadSummary;
}) {
  const threadId = thread.binding.codexThreadId;
  return (
    <button
      aria-current={selected ? "page" : undefined}
      className={cn(
        "group w-full rounded-lg px-2.5 py-2 text-left transition-colors hover:bg-muted/70 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
        selected && "bg-primary/10 text-foreground",
      )}
      data-testid={`code-thread-${threadId}`}
      onClick={() => onSelect(threadId)}
      type="button"
    >
      <span className="block truncate text-sm font-medium">
        {codeThreadLabel(thread)}
      </span>
      <span className="mt-1 flex min-w-0 items-center gap-1.5 text-2xs text-muted-foreground">
        <GitBranch className="h-3 w-3 shrink-0" />
        <span className="truncate">
          {thread.binding.executionMode === "worktree"
            ? "Managed worktree"
            : "Local checkout"}
        </span>
        {thread.unavailable ? (
          <span className="shrink-0 text-destructive">Unavailable</span>
        ) : null}
      </span>
    </button>
  );
}

export function CodeThreadSidebar({
  actionPendingId,
  canCreate,
  creating,
  loading,
  onCreate,
  onOpenPreparation,
  onRefresh,
  onSelectThread,
  preparations,
  selectedThreadId,
  threads,
}: {
  actionPendingId: string | null;
  canCreate: boolean;
  creating: boolean;
  loading: boolean;
  onCreate: () => void;
  onOpenPreparation: (preparation: CodeThreadPreparation) => void;
  onRefresh: () => void;
  onSelectThread: (threadId: string) => void;
  preparations: readonly CodeThreadPreparation[];
  selectedThreadId: string | null;
  threads: readonly CodeBoundThreadSummary[];
}) {
  const groupedPreparations = groupCodePreparations(preparations);
  const orderedPreparations = [
    ...groupedPreparations.starting,
    ...groupedPreparations.prepared,
  ];

  return (
    <aside
      aria-label="Code tasks"
      className="flex min-h-0 w-60 shrink-0 flex-col border-border/60 border-r bg-muted/15"
      data-testid="code-thread-sidebar"
    >
      <div className="flex h-11 items-center gap-2 border-border/60 border-b px-3">
        <h2 className="min-w-0 flex-1 truncate text-sm font-semibold">Tasks</h2>
        <Button
          aria-label="Refresh Code tasks"
          className="h-6 w-6"
          disabled={loading}
          onClick={onRefresh}
          size="icon-xs"
          title="Refresh tasks"
          variant="ghost"
        >
          <RefreshCw
            className={cn(loading && "animate-spin motion-reduce:animate-none")}
          />
        </Button>
      </div>
      <div className="p-2.5">
        <Button
          className="w-full justify-start"
          disabled={!canCreate || creating}
          onClick={onCreate}
          size="sm"
        >
          {creating ? (
            <LoaderCircle className="animate-spin motion-reduce:animate-none" />
          ) : (
            <Plus />
          )}
          New task
        </Button>
      </div>
      <div className="min-h-0 flex-1 space-y-1 overflow-y-auto px-2 pb-3">
        {orderedPreparations.length > 0 ? (
          <div className="space-y-2 pb-2">
            <p className="px-1 text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
              Unfinished
            </p>
            {orderedPreparations.map((preparation) => (
              <PreparationRow
                actionPending={actionPendingId === preparation.preparationId}
                key={preparation.preparationId}
                onOpen={onOpenPreparation}
                preparation={preparation}
              />
            ))}
          </div>
        ) : null}
        <p className="px-1 pt-1 text-2xs font-semibold uppercase tracking-wide text-muted-foreground">
          Recent
        </p>
        {loading && threads.length === 0 ? (
          <div className="flex items-center gap-2 px-2 py-3 text-xs text-muted-foreground">
            <LoaderCircle className="h-4 w-4 animate-spin motion-reduce:animate-none" />
            Loading tasks…
          </div>
        ) : threads.length === 0 ? (
          <p className="px-2 py-3 text-xs text-muted-foreground">
            No Code tasks in this repository yet.
          </p>
        ) : (
          threads.map((thread) => (
            <ThreadRow
              key={thread.binding.codexThreadId}
              onSelect={onSelectThread}
              selected={thread.binding.codexThreadId === selectedThreadId}
              thread={thread}
            />
          ))
        )}
      </div>
    </aside>
  );
}
