import * as React from "react";

import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import type { CodeGitReadyStatus } from "../api/codeGitTypes";
import { codeCommitDialogControls } from "./codeCommitDialogState";

export function CodeCommitDialog({
  blockedReason,
  onCommit,
  onOpenChange,
  open,
  status,
  submitting,
}: {
  blockedReason: string | null;
  onCommit: (message: string) => Promise<unknown>;
  onOpenChange: (open: boolean) => void;
  open: boolean;
  status: CodeGitReadyStatus;
  submitting: boolean;
}) {
  const [message, setMessage] = React.useState("");
  const [error, setError] = React.useState<string | null>(null);
  const identity = status.commitIdentity;
  const binaryCount = status.staged.files.filter((file) => file.binary).length;
  const truncatedCount = status.staged.files.filter(
    (file) => file.truncated,
  ).length;
  const valid =
    message.length > 0 &&
    message === message.trim() &&
    !message.includes("\r") &&
    !/^(co-authored-by|signed-off-by):/im.test(message);
  const controls = codeCommitDialogControls({
    blockedReason,
    messageValid: valid,
    submitting,
  });

  return (
    <Dialog
      onOpenChange={(next) => {
        if (controls.canDismiss) onOpenChange(next);
      }}
      open={open}
    >
      <DialogContent
        onEscapeKeyDown={(event) => {
          event.stopPropagation();
          if (!controls.canDismiss) event.preventDefault();
        }}
        onInteractOutside={(event) => {
          if (!controls.canDismiss) event.preventDefault();
        }}
        showCloseButton={controls.canDismiss}
      >
        <DialogHeader>
          <DialogTitle className="text-balance">
            Commit staged changes
          </DialogTitle>
          <DialogDescription className="text-pretty">
            This creates one local commit on detached HEAD from the staged tree
            only.
          </DialogDescription>
        </DialogHeader>
        <form
          className="space-y-4"
          onSubmit={(event) => {
            event.preventDefault();
            if (blockedReason !== null || submitting) return;
            if (!valid) {
              setError("Enter a trimmed message without identity trailers.");
              return;
            }
            setError(null);
            void onCommit(message).then((receipt) => {
              if (receipt) onOpenChange(false);
            });
          }}
        >
          <div className="space-y-2">
            <label
              className="text-sm font-medium"
              htmlFor="code-commit-message"
            >
              Commit message
            </label>
            <textarea
              aria-describedby={
                blockedReason
                  ? "code-commit-hint code-commit-blocker code-commit-error"
                  : "code-commit-hint code-commit-error"
              }
              autoComplete="off"
              className="min-h-28 w-full resize-y rounded-md border bg-background px-3 py-2 text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring"
              disabled={!controls.canEdit}
              id="code-commit-message"
              maxLength={65_536}
              name="commit-message"
              onChange={(event) => setMessage(event.target.value)}
              value={message}
            />
            <p
              className="text-pretty text-xs text-muted-foreground"
              id="code-commit-hint"
            >
              {status.staged.totalFiles} staged files, {binaryCount} binary,{" "}
              {truncatedCount} truncated patches.
              {identity
                ? ` Author: ${identity.name} <${identity.email}>. Matching Co-authored-by and Signed-off-by trailers will be added.`
                : " Repository-local author identity is unavailable."}
            </p>
            {blockedReason ? (
              <p
                className="text-pretty text-xs text-destructive"
                id="code-commit-blocker"
                role="status"
              >
                {blockedReason}
              </p>
            ) : null}
            <p
              className="text-xs text-destructive"
              id="code-commit-error"
              role={error ? "alert" : undefined}
            >
              {error}
            </p>
          </div>
          <DialogFooter>
            <Button
              disabled={!controls.canDismiss}
              onClick={() => onOpenChange(false)}
              type="button"
              variant="outline"
            >
              Cancel
            </Button>
            <Button disabled={!controls.canSubmit} type="submit">
              {submitting ? "Committing…" : "Commit staged changes"}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
