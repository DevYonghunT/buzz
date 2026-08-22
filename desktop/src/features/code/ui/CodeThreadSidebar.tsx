import {
  CircleDashed,
  GitBranch,
  GitFork,
  LoaderCircle,
  Plus,
  RefreshCw,
  RotateCcw,
  Search,
} from "lucide-react";
import * as React from "react";

import type {
  CodeBoundThreadSummary,
  CodeThreadBindingScope,
  CodeThreadPreparation,
} from "../api/types";
import {
  codeThreadLabel,
  codeThreadLifecycleLabel,
  codeThreadMatchesSearch,
  codeThreadPreparationLabels,
  groupCodePreparations,
} from "../lib/codeWorkspaceView";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { CodeThreadActions } from "./CodeThreadActions";
import { CodeWorktreeInventorySection } from "./CodeWorktreeInventorySection";

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
  const forking = preparation.operation === "fork";
  const labels = codeThreadPreparationLabels(preparation);
  return (
    <div
      className="rounded-lg border border-amber-500/25 bg-amber-500/5 p-2.5"
      data-testid={`code-preparation-${preparation.preparationId}`}
    >
      <div className="flex items-start gap-2">
        {forking ? (
          <GitFork className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
        ) : recovering ? (
          <RotateCcw className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
        ) : (
          <CircleDashed className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
        )}
        <div className="min-w-0 flex-1">
          <p className="text-xs font-medium text-foreground">{labels.title}</p>
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
        {labels.action}
      </Button>
    </div>
  );
}

function ThreadRow({
  actionsReady,
  forkBlocked,
  forkPreparationBlocked,
  lifecycleBlocked,
  onArchive,
  onFork,
  onRename,
  onSelect,
  onUnarchive,
  selected,
  thread,
}: {
  actionsReady: boolean;
  forkBlocked: boolean;
  forkPreparationBlocked: boolean;
  lifecycleBlocked: boolean;
  onArchive: (threadId: string) => Promise<void>;
  onFork: (threadId: string) => Promise<void>;
  onRename: (threadId: string, name: string) => Promise<void>;
  onSelect: (threadId: string) => void;
  onUnarchive: (threadId: string) => Promise<void>;
  selected: boolean;
  thread: CodeBoundThreadSummary;
}) {
  const threadId = thread.binding.codexThreadId;
  return (
    <li
      className={cn(
        "group flex w-full flex-wrap rounded-lg hover:bg-muted/70",
        selected && "bg-primary/10 text-foreground",
      )}
    >
      <button
        aria-current={selected ? "page" : undefined}
        className="min-w-0 flex-1 rounded-lg px-2.5 py-2 text-left focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
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
          <span
            className="shrink-0 rounded-md bg-muted px-1.5 py-0.5"
            data-testid={`code-thread-lifecycle-${threadId}`}
          >
            {codeThreadLifecycleLabel(thread.lifecycle)}
          </span>
          {thread.unavailable ? (
            <span className="shrink-0 text-destructive">Unavailable</span>
          ) : null}
        </span>
      </button>
      <CodeThreadActions
        actionsReady={actionsReady}
        forkBlocked={forkBlocked}
        forkPreparationBlocked={forkPreparationBlocked}
        lifecycleBlocked={lifecycleBlocked}
        onArchive={onArchive}
        onFork={onFork}
        onRename={onRename}
        onUnarchive={onUnarchive}
        thread={thread}
      />
    </li>
  );
}

export function CodeThreadSidebar({
  actionPendingId,
  actionsReady,
  canCreate,
  creating,
  isForkBlocked,
  isLifecycleBlocked,
  loading,
  onArchiveThread,
  onCreate,
  onForkThread,
  onOpenPreparation,
  onRefresh,
  onRenameThread,
  onSelectThread,
  onUnarchiveThread,
  preparations,
  scope,
  selectedThreadId,
  threads,
}: {
  actionPendingId: string | null;
  actionsReady: boolean;
  canCreate: boolean;
  creating: boolean;
  isForkBlocked: (threadId: string) => boolean;
  isLifecycleBlocked: (threadId: string) => boolean;
  loading: boolean;
  onArchiveThread: (threadId: string) => Promise<void>;
  onCreate: () => void;
  onForkThread: (threadId: string) => Promise<void>;
  onOpenPreparation: (preparation: CodeThreadPreparation) => void;
  onRefresh: () => void;
  onRenameThread: (threadId: string, name: string) => Promise<void>;
  onSelectThread: (threadId: string) => void;
  onUnarchiveThread: (threadId: string) => Promise<void>;
  preparations: readonly CodeThreadPreparation[];
  scope: CodeThreadBindingScope;
  selectedThreadId: string | null;
  threads: readonly CodeBoundThreadSummary[];
}) {
  const [searchQuery, setSearchQuery] = React.useState("");
  const groupedPreparations = groupCodePreparations(preparations);
  const orderedPreparations = [
    ...groupedPreparations.starting,
    ...groupedPreparations.prepared,
  ];
  const unfinishedForkSourceIds = React.useMemo(
    () =>
      new Set(
        preparations.flatMap((preparation) =>
          preparation.operation === "fork" && preparation.sourceThreadId
            ? [preparation.sourceThreadId]
            : [],
        ),
      ),
    [preparations],
  );
  const visibleThreads = React.useMemo(
    () =>
      threads.filter((thread) => codeThreadMatchesSearch(thread, searchQuery)),
    [searchQuery, threads],
  );

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
          disabled={
            loading || !actionsReady || creating || actionPendingId !== null
          }
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
      <div className="space-y-2 p-2.5">
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
        <div className="relative">
          <Search className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
          <Input
            aria-label="Search Code tasks"
            className="h-8 pl-8 text-sm"
            data-testid="code-thread-search"
            onChange={(event) => setSearchQuery(event.target.value)}
            placeholder="Search tasks…"
            type="search"
            value={searchQuery}
          />
        </div>
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
        ) : visibleThreads.length === 0 ? (
          <div className="px-2 py-3 text-xs text-muted-foreground">
            <p className="text-pretty">No matching tasks</p>
            <Button
              className="mt-1 -ml-2"
              onClick={() => setSearchQuery("")}
              size="xs"
              variant="ghost"
            >
              Clear search
            </Button>
          </div>
        ) : (
          <ul aria-label="Recent Code tasks" className="space-y-1">
            {visibleThreads.map((thread) => (
              <ThreadRow
                actionsReady={actionsReady}
                forkBlocked={
                  isForkBlocked(thread.binding.codexThreadId) ||
                  unfinishedForkSourceIds.has(thread.binding.codexThreadId)
                }
                forkPreparationBlocked={unfinishedForkSourceIds.has(
                  thread.binding.codexThreadId,
                )}
                key={thread.binding.codexThreadId}
                lifecycleBlocked={isLifecycleBlocked(
                  thread.binding.codexThreadId,
                )}
                onArchive={onArchiveThread}
                onFork={onForkThread}
                onRename={onRenameThread}
                onSelect={onSelectThread}
                onUnarchive={onUnarchiveThread}
                selected={thread.binding.codexThreadId === selectedThreadId}
                thread={thread}
              />
            ))}
          </ul>
        )}
        <CodeWorktreeInventorySection
          actionsReady={actionsReady}
          scope={scope}
        />
      </div>
    </aside>
  );
}
