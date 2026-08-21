import { Ellipsis, Pencil } from "lucide-react";
import * as React from "react";

import type { CodeBoundThreadSummary } from "../api/types";
import { codeThreadLabel } from "../lib/codeWorkspaceView";
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

function renameErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function CodeThreadRenameAction({
  canRename,
  onRename,
  thread,
}: {
  canRename: boolean;
  onRename: (threadId: string, name: string) => Promise<void>;
  thread: CodeBoundThreadSummary;
}) {
  const threadId = thread.binding.codexThreadId;
  const currentLabel = codeThreadLabel(thread);
  const persistedName = thread.thread?.name?.trim() ?? "";
  const [dialogOpen, setDialogOpen] = React.useState(false);
  const [name, setName] = React.useState(currentLabel);
  const [pending, setPending] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const errorId = React.useId();
  const trimmedName = name.trim();
  const renameDisabled =
    pending || trimmedName.length === 0 || trimmedName === persistedName;

  const openRenameDialog = React.useCallback(() => {
    setName(currentLabel);
    setError(null);
    setDialogOpen(true);
  }, [currentLabel]);

  const handleDialogOpenChange = React.useCallback(
    (open: boolean) => {
      if (pending) return;
      setDialogOpen(open);
      if (!open) setError(null);
    },
    [pending],
  );

  const submitRename = React.useCallback(
    async (event: React.FormEvent<HTMLFormElement>) => {
      event.preventDefault();
      if (renameDisabled) return;
      setPending(true);
      setError(null);
      try {
        await onRename(threadId, trimmedName);
        setDialogOpen(false);
      } catch (renameError) {
        setError(renameErrorMessage(renameError));
      } finally {
        setPending(false);
      }
    },
    [onRename, renameDisabled, threadId, trimmedName],
  );

  return (
    <>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <Button
            aria-busy={pending}
            aria-label={`Actions for ${currentLabel}`}
            className="mr-1 size-7 shrink-0 self-center text-muted-foreground"
            data-testid={`code-thread-actions-${threadId}`}
            size="icon-xs"
            type="button"
            variant="ghost"
          >
            <Ellipsis />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="min-w-44">
          <DropdownMenuItem
            disabled={!canRename || thread.unavailable !== null || pending}
            onSelect={() => {
              // Let Radix restore focus after closing the menu before opening
              // the dialog's focus scope.
              globalThis.setTimeout(openRenameDialog, 0);
            }}
          >
            <Pencil />
            Rename task
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <Dialog onOpenChange={handleDialogOpenChange} open={dialogOpen}>
        <DialogContent
          className="max-w-sm"
          data-testid="code-thread-rename-dialog"
          showCloseButton={!pending}
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
                aria-describedby={error ? errorId : undefined}
                aria-invalid={error ? true : undefined}
                autoFocus
                disabled={pending}
                id={`${errorId}-name`}
                onChange={(event) => setName(event.target.value)}
                onFocus={(event) => event.currentTarget.select()}
                value={name}
              />
              {error ? (
                <p
                  className="text-pretty text-xs text-destructive"
                  id={errorId}
                  role="alert"
                >
                  {error}
                </p>
              ) : null}
            </div>
            <DialogFooter>
              <Button
                disabled={pending}
                onClick={() => handleDialogOpenChange(false)}
                type="button"
                variant="outline"
              >
                Cancel
              </Button>
              <Button disabled={renameDisabled} type="submit">
                {pending ? "Saving…" : "Save"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </>
  );
}
