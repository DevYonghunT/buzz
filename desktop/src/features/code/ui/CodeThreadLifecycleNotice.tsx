import {
  Archive,
  ArchiveRestore,
  CircleAlert,
  LoaderCircle,
} from "lucide-react";
import * as React from "react";

import type { CodeThreadLifecycleState } from "../api/types";
import { Button } from "@/shared/ui/button";

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function CodeThreadLifecycleNotice({
  blocked,
  lifecycle,
  onRefresh,
  onUnarchive,
}: {
  blocked: boolean;
  lifecycle: Exclude<CodeThreadLifecycleState, "active">;
  onRefresh: () => Promise<void>;
  onUnarchive: () => Promise<void>;
}) {
  const [pending, setPending] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const run = React.useCallback(
    async (action: () => Promise<void>, allowWhenBlocked = false) => {
      if ((!allowWhenBlocked && blocked) || pending) return;
      setPending(true);
      setError(null);
      try {
        await action();
      } catch (cause) {
        setError(errorMessage(cause));
      } finally {
        setPending(false);
      }
    },
    [blocked, pending],
  );

  const copy =
    lifecycle === "archived"
      ? "This task is archived and read-only. Its worktree and local changes are preserved."
      : lifecycle === "archiving"
        ? "This task is being archived. New Code work remains blocked until its status is confirmed."
        : lifecycle === "unarchiving"
          ? "This task is being restored. New Code work remains blocked until its status is confirmed."
          : "This task status could not be confirmed. Refresh before continuing.";

  return (
    <div
      className="border-border/60 border-t bg-muted/20 px-3 py-3"
      data-testid="code-thread-lifecycle-notice"
    >
      <div className="mx-auto flex max-w-3xl items-center gap-3">
        {lifecycle === "archived" ? (
          <Archive className="size-4 shrink-0 text-muted-foreground" />
        ) : (
          <CircleAlert className="size-4 shrink-0 text-muted-foreground" />
        )}
        <p className="min-w-0 flex-1 text-pretty text-xs text-muted-foreground">
          {copy}
        </p>
        {lifecycle === "archived" ? (
          <Button
            disabled={blocked || pending}
            onClick={() => void run(onUnarchive)}
            size="sm"
            variant="outline"
          >
            {pending ? (
              <LoaderCircle className="animate-spin motion-reduce:animate-none" />
            ) : (
              <ArchiveRestore />
            )}
            {pending ? "Restoring…" : "Unarchive task"}
          </Button>
        ) : (
          <Button
            disabled={pending}
            onClick={() => void run(onRefresh, true)}
            size="sm"
            variant="outline"
          >
            {pending ? (
              <LoaderCircle className="animate-spin motion-reduce:animate-none" />
            ) : null}
            Refresh status
          </Button>
        )}
      </div>
      {error ? (
        <p
          className="mx-auto mt-2 max-w-3xl text-pretty text-xs text-destructive"
          role="alert"
        >
          {error}
        </p>
      ) : null}
    </div>
  );
}
