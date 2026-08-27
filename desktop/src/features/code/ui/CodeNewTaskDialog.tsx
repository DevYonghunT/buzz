import { AlertTriangle, GitBranch, GitFork, LoaderCircle } from "lucide-react";

import type { CodeExecutionMode } from "../api/types";
import {
  type CodeLocalCheckoutSnapshot,
  supportsManagedCodeWorktrees,
} from "../lib/codeTaskCreation";
import type { CodeTaskCreationPhase } from "../state/useCodeTaskCreation";
import { cn } from "@/shared/lib/cn";
import { Alert, AlertDescription, AlertTitle } from "@/shared/ui/alert";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

function pendingLabel(
  phase: CodeTaskCreationPhase,
  executionMode: CodeExecutionMode,
): string | null {
  switch (phase) {
    case "preparing":
      return executionMode === "local"
        ? "Checking local checkout…"
        : "Preparing managed worktree…";
    case "revalidating":
      return "Rechecking local checkout…";
    case "starting":
      return "Creating task…";
    case "refreshing":
      return "Refreshing task state…";
    case "idle":
      return null;
  }
}

function ExecutionModeOption({
  checked,
  description,
  disabled,
  icon: Icon,
  label,
  mode,
  onChange,
  recommended = false,
}: {
  checked: boolean;
  description: string;
  disabled: boolean;
  icon: typeof GitFork;
  label: string;
  mode: CodeExecutionMode;
  onChange: (mode: CodeExecutionMode) => void;
  recommended?: boolean;
}) {
  const descriptionId = `code-task-execution-${mode}-description`;
  return (
    <label
      className={cn(
        "flex cursor-pointer items-start gap-3 rounded-xl border border-border p-3",
        checked && "border-primary bg-primary/5",
        disabled && "cursor-not-allowed opacity-60",
      )}
    >
      <input
        aria-describedby={descriptionId}
        checked={checked}
        className="mt-1 size-4 shrink-0 accent-primary"
        disabled={disabled}
        name="code-task-execution-mode"
        onChange={() => onChange(mode)}
        type="radio"
        value={mode}
      />
      <Icon aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
      <span className="min-w-0 flex-1">
        <span className="flex flex-wrap items-center gap-2 text-sm font-medium">
          {label}
          {recommended ? (
            <span className="rounded-md bg-primary/10 px-1.5 py-0.5 text-2xs text-primary">
              Recommended
            </span>
          ) : null}
        </span>
        <span
          className="mt-1 block text-pretty text-xs text-muted-foreground"
          id={descriptionId}
        >
          {description}
        </span>
      </span>
    </label>
  );
}

function LocalCheckoutWarning({
  changed,
  snapshot,
}: {
  changed: boolean;
  snapshot: CodeLocalCheckoutSnapshot | null;
}) {
  return (
    <Alert data-testid="code-local-checkout-warning" variant="destructive">
      <div className="flex items-start gap-2.5">
        <AlertTriangle
          aria-hidden="true"
          className="mt-0.5 size-4 shrink-0 text-destructive"
        />
        <div className="min-w-0 flex-1">
          <AlertTitle className="text-balance">Existing checkout</AlertTitle>
          <AlertDescription className="text-pretty text-muted-foreground">
            Changes from this task apply directly to your existing checkout.
            SchoolX will not switch branches, reset, or clean it.
          </AlertDescription>
          {snapshot ? (
            <dl className="mt-3 grid grid-cols-2 gap-x-3 gap-y-1 rounded-lg border border-destructive/20 bg-background/60 p-2.5">
              <dt className="text-muted-foreground">Current branch</dt>
              <dd
                className="truncate text-right font-mono"
                data-testid="code-local-checkout-branch"
              >
                {snapshot.branch ?? "Detached HEAD"}
              </dd>
              <dt className="text-muted-foreground">Working tree</dt>
              <dd
                className={cn(
                  "text-right font-medium",
                  snapshot.dirty && "text-destructive",
                )}
                data-testid="code-local-checkout-dirty"
              >
                {snapshot.dirty ? "Uncommitted changes present" : "Clean"}
              </dd>
            </dl>
          ) : (
            <p className="mt-2 text-pretty text-muted-foreground">
              Review the native-verified branch and working-tree state before
              the task starts.
            </p>
          )}
          {changed ? (
            <p
              className="mt-2 text-pretty font-medium text-destructive"
              data-testid="code-local-checkout-drift"
            >
              The checkout changed during review. Check the latest state above,
              then confirm again.
            </p>
          ) : null}
        </div>
      </div>
    </Alert>
  );
}

export function CodeNewTaskDialog({
  enabled,
  error,
  executionMode,
  localSnapshot,
  localSnapshotChanged,
  onExecutionModeChange,
  onRestoreFocus,
  onOpenChange,
  onSubmit,
  open,
  pending,
  phase,
  preparationReady,
  recoveryRequired,
}: {
  enabled: boolean;
  error: string | null;
  executionMode: CodeExecutionMode;
  localSnapshot: CodeLocalCheckoutSnapshot | null;
  localSnapshotChanged: boolean;
  onExecutionModeChange: (mode: CodeExecutionMode) => void;
  onRestoreFocus: () => void;
  onOpenChange: (open: boolean) => void;
  onSubmit: () => void;
  open: boolean;
  pending: boolean;
  phase: CodeTaskCreationPhase;
  preparationReady: boolean;
  recoveryRequired: boolean;
}) {
  const managedWorktreesSupported = supportsManagedCodeWorktrees();
  const modeLocked = pending || preparationReady;
  const progressLabel = pendingLabel(phase, executionMode);
  const submitLabel =
    executionMode === "worktree"
      ? "Create task"
      : localSnapshot === null
        ? "Review local checkout"
        : "Create task in local checkout";

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        aria-busy={pending}
        className="max-w-xl"
        data-testid="code-new-task-dialog"
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          onRestoreFocus();
        }}
        showCloseButton={!pending}
      >
        <DialogHeader>
          <DialogTitle className="text-balance">New Code task</DialogTitle>
          <DialogDescription className="text-pretty">
            {managedWorktreesSupported
              ? "Choose where Codex can change files for this task. Every new task starts with a managed worktree selected."
              : "On Windows, SchoolX Code works in the local checkout after you review its current state."}
          </DialogDescription>
        </DialogHeader>

        <form
          className="space-y-4"
          onSubmit={(event) => {
            event.preventDefault();
            onSubmit();
          }}
        >
          <fieldset className="space-y-2" disabled={modeLocked}>
            <legend className="mb-2 text-sm font-medium">
              Execution location
            </legend>
            <ExecutionModeOption
              checked={executionMode === "worktree"}
              description={
                managedWorktreesSupported
                  ? "Work in an isolated checkout managed by SchoolX. Your existing checkout stays untouched."
                  : "Managed worktrees are not available on Windows."
              }
              disabled={modeLocked || !managedWorktreesSupported}
              icon={GitFork}
              label="Managed worktree"
              mode="worktree"
              onChange={onExecutionModeChange}
              recommended={managedWorktreesSupported}
            />
            <ExecutionModeOption
              checked={executionMode === "local"}
              description="Work directly in the checkout that is already open on this computer."
              disabled={modeLocked}
              icon={GitBranch}
              label="Local checkout"
              mode="local"
              onChange={onExecutionModeChange}
              recommended={!managedWorktreesSupported}
            />
          </fieldset>

          {executionMode === "local" ? (
            <LocalCheckoutWarning
              changed={localSnapshotChanged}
              snapshot={localSnapshot}
            />
          ) : null}

          {error ? (
            <Alert data-testid="code-task-creation-error" variant="destructive">
              <AlertTitle className="text-balance">
                Couldn’t create task
              </AlertTitle>
              <AlertDescription className="text-pretty text-destructive">
                {error}
                {recoveryRequired ? (
                  <span className="mt-2 block text-muted-foreground">
                    Close this dialog, then continue from Unfinished so native
                    state determines whether to start or recover the task.
                  </span>
                ) : null}
              </AlertDescription>
            </Alert>
          ) : null}

          <DialogFooter>
            <DialogClose asChild>
              <Button disabled={pending} type="button" variant="outline">
                {executionMode === "local" && preparationReady
                  ? "Continue later"
                  : "Cancel"}
              </Button>
            </DialogClose>
            <Button
              aria-busy={pending}
              data-testid="code-new-task-submit"
              disabled={!enabled || pending}
              type="submit"
            >
              {pending ? (
                <LoaderCircle
                  aria-hidden="true"
                  className="animate-spin motion-reduce:animate-none"
                />
              ) : null}
              {progressLabel ?? submitLabel}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
