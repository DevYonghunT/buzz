import {
  FileWarning,
  KeyRound,
  LoaderCircle,
  TerminalSquare,
} from "lucide-react";
import * as React from "react";

import { CodePermissionDisplaySchema } from "../api/schemas";
import type {
  CodeApprovalResponse,
  CodePermissionDisplay,
  CodePermissionIntent,
  CodePermissionScope,
  CodePermissionSpecialPathDisplay,
  JsonObject,
  JsonValue,
} from "../api/types";
import type { CodePendingApproval } from "../state/codeSessionReducer";
import { Button } from "@/shared/ui/button";

function stringValue(value: JsonValue | undefined): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

export function codePermissionDisplayFromRequest(
  request: JsonObject,
): CodePermissionDisplay | null {
  if ("permissions" in request) return null;
  const parsed = CodePermissionDisplaySchema.safeParse(
    request.permissionDisplay,
  );
  return parsed.success ? parsed.data : null;
}

export function isCodePermissionDisplayGrantable(
  display: CodePermissionDisplay | null,
): boolean {
  if (display?.grantable !== true) return false;
  const fileSystem = display.fileSystem;
  const legacyPaths = [
    ...(fileSystem?.read ?? []),
    ...(fileSystem?.write ?? []),
  ];
  if (!legacyPaths.every(isExactDisplayText)) return false;
  const entries = fileSystem?.entries ?? [];
  if (
    !entries.every(({ path }) => {
      if (path.type === "path") return isExactDisplayText(path.path);
      if (path.type === "globPattern") {
        return isExactDisplayText(path.pattern);
      }
      if (path.value.kind === "project_roots") {
        return (
          path.value.subpath === null || isExactDisplayText(path.value.subpath)
        );
      }
      if (path.value.kind === "unknown") {
        return (
          isExactDisplayText(path.value.path) &&
          (path.value.subpath === null ||
            isExactDisplayText(path.value.subpath))
        );
      }
      return true;
    })
  ) {
    return false;
  }
  return (
    display.network?.enabled === true ||
    legacyPaths.length > 0 ||
    entries.length > 0
  );
}

function isExactDisplayText(value: string): boolean {
  return value.length > 0 && !value.includes("[REDACTED]");
}

function specialPathLabel(value: CodePermissionSpecialPathDisplay): string {
  if (value.kind === "project_roots") {
    return value.subpath ? `${value.kind}/${value.subpath}` : value.kind;
  }
  if (value.kind === "unknown") {
    return value.subpath
      ? `${value.kind}: ${value.path}/${value.subpath}`
      : `${value.kind}: ${value.path}`;
  }
  return value.kind;
}

/** Stable human-readable rows for every display-only authority scope. */
export function codePermissionDisplayRows(
  display: CodePermissionDisplay,
): string[] {
  const rows: string[] = [];
  if (display.network !== null) {
    const enabled =
      display.network.enabled === null
        ? "unspecified"
        : display.network.enabled
          ? "enabled"
          : "disabled";
    rows.push(`Network: ${enabled}`);
  }
  const fileSystem = display.fileSystem;
  if (fileSystem === null) return rows;
  for (const path of fileSystem.read ?? []) {
    rows.push(`Filesystem read path: ${path}`);
  }
  for (const path of fileSystem.write ?? []) {
    rows.push(`Filesystem write path: ${path}`);
  }
  for (const entry of fileSystem.entries ?? []) {
    const path =
      entry.path.type === "path"
        ? `path: ${entry.path.path}`
        : entry.path.type === "globPattern"
          ? `glob: ${entry.path.pattern}`
          : `special: ${specialPathLabel(entry.path.value)}`;
    rows.push(`Filesystem ${entry.access} ${path}`);
  }
  if (fileSystem.globScanMaxDepth !== null) {
    rows.push(`Filesystem glob scan max depth: ${fileSystem.globScanMaxDepth}`);
  }
  return rows;
}

/** Build an opaque permission response without copying display data. */
export function codePermissionResponse(
  intent: CodePermissionIntent,
  scope: CodePermissionScope,
): CodeApprovalResponse {
  return { type: "permissions", intent, scope };
}

function approvalCopy(
  approval: CodePendingApproval,
  permissionDisplay: CodePermissionDisplay | null,
): {
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
  const scopeCount = permissionDisplay
    ? codePermissionDisplayRows(permissionDisplay).length
    : 0;
  return {
    title: "Permission approval",
    subject:
      scopeCount > 0
        ? `Review ${scopeCount} requested permission scopes`
        : "Review requested permissions",
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
  const permissionDisplay =
    approval.approvalKind === "permissions"
      ? codePermissionDisplayFromRequest(approval.request)
      : null;
  const permissionRows = permissionDisplay
    ? codePermissionDisplayRows(permissionDisplay)
    : [];
  const permissionGrantable =
    isCodePermissionDisplayGrantable(permissionDisplay);
  const copy = approvalCopy(approval, permissionDisplay);
  const Icon =
    approval.approvalKind === "commandExecution"
      ? TerminalSquare
      : approval.approvalKind === "fileChange"
        ? FileWarning
        : KeyRound;
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
      if (!permissionGrantable) return;
      void respond(codePermissionResponse("grant", scope));
      return;
    }
    void respond({
      type: "decision",
      decision: scope === "session" ? "acceptForSession" : "accept",
    });
  };

  const decline = () => {
    if (approval.approvalKind === "permissions") {
      void respond(codePermissionResponse("decline", "turn"));
      return;
    }
    void respond({ type: "decision", decision: "decline" });
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
          {approval.approvalKind === "permissions" && permissionRows.length ? (
            <ul
              className="mt-2 space-y-1 text-xs text-foreground"
              data-testid="code-permission-scopes"
            >
              {permissionRows.map((row) => (
                <li className="break-words font-mono" key={row}>
                  {row}
                </li>
              ))}
            </ul>
          ) : null}
          {approval.approvalKind === "permissions" && !permissionGrantable ? (
            <p
              className="mt-2 text-xs text-destructive"
              data-testid="code-permission-ungrantable"
            >
              Permission details could not be verified. Decline this request.
            </p>
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
        <Button
          disabled={!canRespond || pending}
          onClick={decline}
          size="xs"
          variant="ghost"
        >
          Decline
        </Button>
        <Button
          disabled={
            !canRespond ||
            pending ||
            (approval.approvalKind === "permissions" && !permissionGrantable)
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
            (approval.approvalKind === "permissions" && !permissionGrantable)
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
