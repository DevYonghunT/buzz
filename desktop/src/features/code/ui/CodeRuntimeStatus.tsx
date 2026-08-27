import {
  AlertTriangle,
  CheckCircle2,
  LoaderCircle,
  Play,
  RotateCw,
} from "lucide-react";

import type { CodeReplayState } from "../state/codeSessionReducer";
import type { CodeRuntimeStatus as RuntimeStatus } from "../api/types";
import { codeRuntimePresentation } from "../lib/codeWorkspaceView";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";

export function CodeRuntimeStatus({
  error,
  onRefresh,
  onRetrySync,
  onStart,
  pending,
  replay,
  status,
  subscriptionError,
}: {
  error: string | null;
  onRefresh: () => void;
  onRetrySync: () => void;
  onStart: () => void;
  pending: boolean;
  replay: CodeReplayState;
  status: RuntimeStatus | null;
  subscriptionError: string | null;
}) {
  const presentation = codeRuntimePresentation(status?.phase ?? null);
  const Icon =
    presentation.tone === "ready"
      ? CheckCircle2
      : presentation.tone === "error"
        ? AlertTriangle
        : LoaderCircle;
  const eventStateNeedsAttention =
    replay.status === "invalid" ||
    replay.status === "truncated" ||
    subscriptionError !== null;
  const eventSyncPending =
    status?.phase === "ready" &&
    (replay.status === "idle" || replay.status === "synchronizing");
  const canStart = status?.phase === "stopped" || status?.phase === "failed";
  const needsInstallRecheck = status?.phase === "notInstalled";

  return (
    <div
      aria-live="polite"
      className="space-y-2 border-border/60 border-b px-3 py-3"
    >
      <div className="flex items-start gap-2">
        <Icon
          aria-hidden="true"
          className={cn(
            "mt-0.5 h-4 w-4 shrink-0",
            presentation.tone === "ready" && "text-emerald-500",
            presentation.tone === "error" && "text-destructive",
            presentation.tone === "pending" &&
              "animate-spin text-primary motion-reduce:animate-none",
            presentation.tone === "neutral" && "text-muted-foreground",
          )}
        />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <p className="truncate text-sm font-medium text-foreground">
              {presentation.label}
            </p>
            {status?.version ? (
              <span className="truncate text-2xs text-muted-foreground">
                {status.version}
              </span>
            ) : null}
          </div>
          <p className="mt-0.5 text-xs text-muted-foreground">
            {error ??
              status?.lastError ??
              (eventSyncPending
                ? "Synchronizing this project's Code activity…"
                : presentation.description)}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          {canStart ? (
            <Button
              disabled={pending}
              onClick={onStart}
              size="xs"
              variant="outline"
            >
              {pending ? (
                <LoaderCircle className="animate-spin motion-reduce:animate-none" />
              ) : (
                <Play />
              )}
              {status.phase === "failed" ? "Retry" : "Start"}
            </Button>
          ) : null}
          {needsInstallRecheck ? (
            <Button
              disabled={pending}
              onClick={onRefresh}
              size="xs"
              variant="outline"
            >
              <RotateCw
                className={cn(
                  pending && "animate-spin motion-reduce:animate-none",
                )}
              />
              {pending ? "Checking…" : "Check again"}
            </Button>
          ) : (
            <Button
              aria-label="Refresh Codex runtime status"
              className="h-6 w-6"
              disabled={pending}
              onClick={onRefresh}
              size="icon-xs"
              title="Refresh runtime status"
              variant="ghost"
            >
              <RotateCw
                className={cn(
                  pending && "animate-spin motion-reduce:animate-none",
                )}
              />
            </Button>
          )}
        </div>
      </div>

      {eventStateNeedsAttention ? (
        <div className="rounded-md border border-amber-500/30 bg-amber-500/10 p-2 text-xs text-foreground">
          <p>
            {subscriptionError ??
              "Live activity may be incomplete. Approval actions are paused until a full sync succeeds."}
          </p>
          <Button
            className="mt-2 h-6"
            onClick={onRetrySync}
            size="xs"
            variant="outline"
          >
            Sync activity
          </Button>
        </div>
      ) : null}
    </div>
  );
}
