import {
  FileWarning,
  KeyRound,
  LoaderCircle,
  TerminalSquare,
} from "lucide-react";
import * as React from "react";

import type { CodeApprovalResponse, JsonObject, JsonValue } from "../api/types";
import type { CodePendingApproval } from "../state/codeSessionReducer";
import { Button } from "@/shared/ui/button";

function stringValue(value: JsonValue | undefined): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function objectValue(value: JsonValue | undefined): JsonObject | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value
    : null;
}

function approvalCopy(approval: CodePendingApproval): {
  title: string;
  subject: string;
  detail: string | null;
} {
  const request = approval.request;
  const reason = stringValue(request.reason);
  if (approval.approvalKind === "commandExecution") {
    return {
      title: "Command approval",
      subject: stringValue(request.command) ?? "Run a requested command",
      detail: reason ?? stringValue(request.cwd),
    };
  }
  if (approval.approvalKind === "fileChange") {
    return {
      title: "File change approval",
      subject: "Apply the proposed file changes",
      detail: reason,
    };
  }
  const permissions = objectValue(request.permissions);
  return {
    title: "Permission approval",
    subject: permissions
      ? `Allow ${Object.keys(permissions).join(", ")}`
      : "Allow requested permissions",
    detail: reason ?? stringValue(request.cwd),
  };
}

export function CodeApprovalCard({
  approval,
  canRespond,
  onRespond,
}: {
  approval: CodePendingApproval;
  canRespond: boolean;
  onRespond: (response: CodeApprovalResponse) => Promise<void>;
}) {
  const [pending, setPending] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const copy = approvalCopy(approval);
  const Icon =
    approval.approvalKind === "commandExecution"
      ? TerminalSquare
      : approval.approvalKind === "fileChange"
        ? FileWarning
        : KeyRound;
  const requestedPermissions = objectValue(approval.request.permissions);

  const respond = React.useCallback(
    async (response: CodeApprovalResponse) => {
      if (!canRespond || pending) return;
      setPending(true);
      setError(null);
      try {
        await onRespond(response);
      } catch (responseError) {
        setError(
          responseError instanceof Error
            ? responseError.message
            : String(responseError),
        );
      } finally {
        setPending(false);
      }
    },
    [canRespond, onRespond, pending],
  );

  const allow = (scope: "turn" | "session") => {
    if (approval.approvalKind === "permissions") {
      if (requestedPermissions === null) return;
      void respond({
        type: "permissions",
        permissions: requestedPermissions,
        scope,
        strictAutoReview: true,
      });
      return;
    }
    void respond({
      type: "decision",
      decision: scope === "session" ? "acceptForSession" : "accept",
    });
  };

  return (
    <section
      aria-label={copy.title}
      className="rounded-xl border border-amber-500/35 bg-amber-500/10 p-3"
      data-testid={`code-approval-${approval.itemId}`}
    >
      <div className="flex items-start gap-2.5">
        <Icon className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
        <div className="min-w-0 flex-1">
          <p className="text-xs font-semibold text-foreground">{copy.title}</p>
          <p className="mt-1 break-words font-mono text-xs text-foreground">
            {copy.subject}
          </p>
          {copy.detail ? (
            <p className="mt-1 text-xs text-muted-foreground">{copy.detail}</p>
          ) : null}
        </div>
      </div>
      {!canRespond ? (
        <p className="mt-2 text-xs text-muted-foreground">
          This request is no longer active.
        </p>
      ) : null}
      {error ? <p className="mt-2 text-xs text-destructive">{error}</p> : null}
      <div className="mt-3 flex flex-wrap justify-end gap-1.5">
        {approval.approvalKind !== "permissions" ? (
          <Button
            disabled={!canRespond || pending}
            onClick={() =>
              void respond({ type: "decision", decision: "decline" })
            }
            size="xs"
            variant="ghost"
          >
            Decline
          </Button>
        ) : null}
        <Button
          disabled={
            !canRespond ||
            pending ||
            (approval.approvalKind === "permissions" &&
              requestedPermissions === null)
          }
          onClick={() => allow("turn")}
          size="xs"
          variant="outline"
        >
          {pending ? (
            <LoaderCircle className="animate-spin motion-reduce:animate-none" />
          ) : null}
          Allow once
        </Button>
        <Button
          disabled={
            !canRespond ||
            pending ||
            (approval.approvalKind === "permissions" &&
              requestedPermissions === null)
          }
          onClick={() => allow("session")}
          size="xs"
        >
          Allow for session
        </Button>
      </div>
    </section>
  );
}
