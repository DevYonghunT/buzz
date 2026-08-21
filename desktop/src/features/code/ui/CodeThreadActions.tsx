import {
  Archive,
  ArchiveRestore,
  Ellipsis,
  GitFork,
  LoaderCircle,
  Pencil,
} from "lucide-react";
import * as React from "react";

import type { CodeBoundThreadSummary } from "../api/types";
import {
  codeThreadLabel,
  codeThreadLifecycleCapabilities,
} from "../lib/codeWorkspaceView";
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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Input } from "@/shared/ui/input";

function actionErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function CodeThreadActions({
  actionsReady,
  forkBlocked,
  forkPreparationBlocked,
  lifecycleBlocked,
  onArchive,
  onFork,
  onRename,
  onUnarchive,
  thread,
}: {
  actionsReady: boolean;
  forkBlocked: boolean;
  forkPreparationBlocked: boolean;
  lifecycleBlocked: boolean;
  onArchive: (threadId: string) => Promise<void>;
  onFork: (threadId: string) => Promise<void>;
  onRename: (threadId: string, name: string) => Promise<void>;
  onUnarchive: (threadId: string) => Promise<void>;
  thread: CodeBoundThreadSummary;
}) {
  const threadId = thread.binding.codexThreadId;
  const currentLabel = codeThreadLabel(thread);
  const capabilities = codeThreadLifecycleCapabilities(thread.lifecycle);
  const [renameDialogOpen, setRenameDialogOpen] = React.useState(false);
  const [archiveDialogOpen, setArchiveDialogOpen] = React.useState(false);
  const [name, setName] = React.useState(currentLabel);
  const [renamePending, setRenamePending] = React.useState(false);
  const [lifecyclePending, setLifecyclePending] = React.useState(false);
  const [renameError, setRenameError] = React.useState<string | null>(null);
  const [lifecycleError, setLifecycleError] = React.useState<string | null>(
    null,
  );
  const errorId = React.useId();
  const trimmedName = name.trim();
  const anyPending = renamePending || lifecyclePending || lifecycleBlocked;
  const renameDisabled =
    anyPending || trimmedName.length === 0 || trimmedName === currentLabel;
  const canRename =
    actionsReady &&
    capabilities.canRename &&
    thread.unavailable === null &&
    !anyPending;
  const canFork =
    actionsReady &&
    capabilities.canFork &&
    thread.binding.executionMode === "worktree" &&
    thread.binding.worktreeId !== null &&
    thread.unavailable === null &&
    !forkBlocked &&
    !anyPending;
  const hasActions =
    capabilities.canRename ||
    capabilities.canFork ||
    capabilities.canArchive ||
    capabilities.canUnarchive;

  React.useEffect(() => {
    if (!forkPreparationBlocked) setLifecycleError(null);
  }, [forkPreparationBlocked]);

  const openRenameDialog = React.useCallback(() => {
    setName(currentLabel);
    setRenameError(null);
    setLifecycleError(null);
    setRenameDialogOpen(true);
  }, [currentLabel]);

  const handleRenameDialogOpenChange = React.useCallback(
    (open: boolean) => {
      if (renamePending) return;
      setRenameDialogOpen(open);
      if (!open) setRenameError(null);
    },
    [renamePending],
  );

  const submitRename = React.useCallback(
    async (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      if (renameDisabled || !canRename) return;
      setRenamePending(true);
      setRenameError(null);
      try {
        await onRename(threadId, trimmedName);
        setRenameDialogOpen(false);
      } catch (error) {
        setRenameError(actionErrorMessage(error));
      } finally {
        setRenamePending(false);
      }
    },
    [canRename, onRename, renameDisabled, threadId, trimmedName],
  );

  const runLifecycleAction = React.useCallback(
    async (action: "archive" | "fork" | "unarchive") => {
      if (!actionsReady || anyPending) return;
      setLifecyclePending(true);
      setLifecycleError(null);
      try {
        if (action === "archive") await onArchive(threadId);
        else if (action === "fork") await onFork(threadId);
        else await onUnarchive(threadId);
        setArchiveDialogOpen(false);
      } catch (error) {
        setLifecycleError(actionErrorMessage(error));
      } finally {
        setLifecyclePending(false);
      }
    },
    [actionsReady, anyPending, onArchive, onFork, onUnarchive, threadId],
  );

  if (!hasActions) return null;

  return (
    <>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <Button
            aria-busy={anyPending}
            aria-label={`Actions for ${currentLabel}`}
            className="mr-1 size-7 shrink-0 self-center text-muted-foreground"
            data-testid={`code-thread-actions-${threadId}`}
            disabled={!actionsReady || anyPending}
            size="icon-xs"
            type="button"
            variant="ghost"
          >
            {anyPending ? (
              <LoaderCircle className="animate-spin motion-reduce:animate-none" />
            ) : (
              <Ellipsis />
            )}
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="min-w-44">
          <DropdownMenuItem
            disabled={!canRename}
            onSelect={() => {
              globalThis.setTimeout(openRenameDialog, 0);
            }}
          >
            <Pencil />
            Rename task
          </DropdownMenuItem>
          {capabilities.canFork ? (
            <DropdownMenuItem
              disabled={!canFork}
              onSelect={() => {
                void runLifecycleAction("fork");
              }}
            >
              <GitFork />
              Fork task
            </DropdownMenuItem>
          ) : null}
          {capabilities.canArchive ? (
            <DropdownMenuItem
              className="text-destructive focus:text-destructive"
              disabled={!actionsReady || anyPending}
              onSelect={() => {
                setLifecycleError(null);
                globalThis.setTimeout(() => setArchiveDialogOpen(true), 0);
              }}
            >
              <Archive />
              Archive task
            </DropdownMenuItem>
          ) : null}
          {capabilities.canUnarchive ? (
            <DropdownMenuItem
              disabled={!actionsReady || anyPending}
              onSelect={() => {
                void runLifecycleAction("unarchive");
              }}
            >
              <ArchiveRestore />
              Unarchive task
            </DropdownMenuItem>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>

      {lifecycleError && !archiveDialogOpen ? (
        <p
          className="basis-full px-2.5 pb-2 text-pretty text-xs text-destructive"
          role="alert"
        >
          {lifecycleError}
        </p>
      ) : null}

      <Dialog
        onOpenChange={handleRenameDialogOpenChange}
        open={renameDialogOpen}
      >
        <DialogContent
          className="max-w-sm"
          data-testid="code-thread-rename-dialog"
          showCloseButton={!renamePending}
        >
          <DialogHeader>
            <DialogTitle className="text-balance">Rename task</DialogTitle>
            <DialogDescription className="text-pretty">
              Give this Code task a name that is easy to find later.
            </DialogDescription>
          </DialogHeader>
          <form className="space-y-4" onSubmit={submitRename}>
            <div className="space-y-2">
              <label
                className="text-sm font-medium"
                htmlFor={`${errorId}-name`}
              >
                Task name
              </label>
              <Input
                aria-describedby={renameError ? errorId : undefined}
                aria-invalid={renameError ? true : undefined}
                autoFocus
                disabled={renamePending}
                id={`${errorId}-name`}
                onChange={(event) => setName(event.target.value)}
                onFocus={(event) => event.currentTarget.select()}
                value={name}
              />
              {renameError ? (
                <p
                  className="text-pretty text-xs text-destructive"
                  id={errorId}
                  role="alert"
                >
                  {renameError}
                </p>
              ) : null}
            </div>
            <DialogFooter>
              <Button
                disabled={renamePending}
                onClick={() => handleRenameDialogOpenChange(false)}
                type="button"
                variant="outline"
              >
                Cancel
              </Button>
              <Button disabled={renameDisabled || !canRename} type="submit">
                {renamePending ? "Saving…" : "Save"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

      <AlertDialog
        onOpenChange={(open) => {
          if (!lifecyclePending) setArchiveDialogOpen(open);
        }}
        open={archiveDialogOpen}
      >
        <AlertDialogContent data-testid="code-thread-archive-dialog">
          <AlertDialogHeader>
            <AlertDialogTitle className="text-balance">
              Archive this task?
            </AlertDialogTitle>
            <AlertDialogDescription className="text-pretty">
              This ends its terminal session and pauses new Code work. The
              binding, worktree, and local changes are preserved, and you can
              unarchive the task later.
            </AlertDialogDescription>
          </AlertDialogHeader>
          {lifecycleError ? (
            <p className="text-pretty text-sm text-destructive" role="alert">
              {lifecycleError}
            </p>
          ) : null}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={lifecyclePending}>
              Cancel
            </AlertDialogCancel>
            <AlertDialogAction
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
              disabled={lifecyclePending}
              onClick={(event) => {
                event.preventDefault();
                void runLifecycleAction("archive");
              }}
            >
              {lifecyclePending ? (
                <LoaderCircle className="animate-spin motion-reduce:animate-none" />
              ) : (
                <Archive />
              )}
              {lifecyclePending ? "Archiving…" : "Archive task"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
