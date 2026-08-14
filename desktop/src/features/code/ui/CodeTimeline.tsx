import {
  AlertTriangle,
  Bot,
  CheckCircle2,
  CircleDashed,
  FileCode2,
  ListChecks,
  TerminalSquare,
  UserRound,
} from "lucide-react";
import * as React from "react";

import type { CodeApprovalResponse } from "../api/types";
import type { CodeTimelineRow } from "../lib/codeTimeline";
import {
  codeApprovalIdentityKey,
  type CodePendingApproval,
} from "../state/codeSessionReducer";
import { cn } from "@/shared/lib/cn";
import { CodeApprovalCard } from "./CodeApprovalCard";

function TimelineRow({ row }: { row: CodeTimelineRow }) {
  if (row.kind === "turnStatus") {
    return (
      <div className="flex items-center gap-2 py-1 text-2xs text-muted-foreground">
        <span className="h-px flex-1 bg-border/60" />
        <CheckCircle2 className="h-3 w-3" />
        <span>{row.status}</span>
        <span className="h-px flex-1 bg-border/60" />
      </div>
    );
  }

  if (row.kind === "user") {
    return (
      <article className="ml-auto max-w-[85%] rounded-xl bg-primary px-3 py-2 text-primary-foreground shadow-xs">
        <div className="mb-1 flex items-center justify-end gap-1.5 text-2xs opacity-75">
          <UserRound className="h-3 w-3" />
          You
        </div>
        <p className="whitespace-pre-wrap break-words text-sm">{row.text}</p>
      </article>
    );
  }

  if (row.kind === "warning" || row.kind === "error") {
    return (
      <article
        className={cn(
          "rounded-lg border p-3 text-xs",
          row.kind === "error"
            ? "border-destructive/35 bg-destructive/10 text-destructive"
            : "border-amber-500/35 bg-amber-500/10 text-foreground",
        )}
      >
        <div className="flex items-start gap-2">
          <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" />
          <p className="whitespace-pre-wrap break-words">{row.message}</p>
        </div>
      </article>
    );
  }

  if (row.kind === "agent") {
    return (
      <article className="max-w-3xl rounded-xl border border-border/60 bg-card/40 p-3">
        <div className="mb-1.5 flex items-center gap-1.5 text-2xs font-medium text-muted-foreground">
          <Bot className="h-3.5 w-3.5" />
          Codex
          {row.streaming ? (
            <CircleDashed className="ml-1 h-3 w-3 animate-spin motion-reduce:animate-none" />
          ) : null}
        </div>
        <p className="whitespace-pre-wrap break-words text-sm leading-relaxed">
          {row.text}
        </p>
      </article>
    );
  }

  if (row.kind === "plan") {
    return (
      <article className="max-w-3xl rounded-xl border border-border/60 bg-muted/20 p-3">
        <div className="mb-1.5 flex items-center gap-1.5 text-xs font-semibold">
          <ListChecks className="h-4 w-4 text-muted-foreground" />
          Plan
          {row.streaming ? (
            <CircleDashed className="h-3 w-3 animate-spin text-muted-foreground motion-reduce:animate-none" />
          ) : null}
        </div>
        {row.text ? (
          <p className="whitespace-pre-wrap break-words text-xs text-muted-foreground">
            {row.text}
          </p>
        ) : null}
        {row.steps.length > 0 ? (
          <ol className="mt-2 space-y-1 pl-5 text-xs">
            {row.steps.map((step) => (
              <li
                className="list-decimal"
                key={`${step.text}:${step.status ?? ""}`}
              >
                {step.text}
                {step.status ? (
                  <span className="ml-1.5 text-2xs text-muted-foreground">
                    {step.status}
                  </span>
                ) : null}
              </li>
            ))}
          </ol>
        ) : null}
      </article>
    );
  }

  if (row.kind === "commandOutput") {
    return (
      <article className="max-w-3xl overflow-hidden rounded-xl border border-border/60 bg-muted/20">
        <div className="flex items-center gap-2 border-border/60 border-b px-3 py-2 text-xs font-medium">
          <TerminalSquare className="h-4 w-4 text-muted-foreground" />
          <span className="min-w-0 flex-1 truncate font-mono">
            {row.command ?? "Command output"}
          </span>
          {row.status ? (
            <span className="text-2xs text-muted-foreground">{row.status}</span>
          ) : null}
        </div>
        {row.output ? (
          <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words p-3 font-mono text-xs text-foreground">
            {row.output}
          </pre>
        ) : (
          <p className="px-3 py-2 text-xs text-muted-foreground">
            Waiting for output…
          </p>
        )}
      </article>
    );
  }

  if (row.kind !== "fileChange") return null;

  return (
    <article className="max-w-3xl rounded-xl border border-border/60 bg-muted/20 p-3">
      <div className="flex items-center gap-2 text-xs font-medium">
        <FileCode2 className="h-4 w-4 text-muted-foreground" />
        File changes
        {row.status ? (
          <span className="text-2xs text-muted-foreground">{row.status}</span>
        ) : null}
      </div>
      {row.changes.length > 0 ? (
        <ul className="mt-2 space-y-1 font-mono text-xs">
          {row.changes.map((change) => (
            <li className="flex min-w-0 items-center gap-2" key={change.path}>
              <span className="min-w-0 flex-1 truncate">{change.path}</span>
              {change.changeType ? (
                <span className="text-2xs text-muted-foreground">
                  {change.changeType}
                </span>
              ) : null}
            </li>
          ))}
        </ul>
      ) : (
        <p className="mt-1 text-xs text-muted-foreground">
          Codex reported a file update.
        </p>
      )}
    </article>
  );
}

export function CodeTimeline({
  approvals,
  canRespond,
  loading,
  onRespond,
  rows,
}: {
  approvals: readonly CodePendingApproval[];
  canRespond: (approval: CodePendingApproval) => boolean;
  loading: boolean;
  onRespond: (
    approval: CodePendingApproval,
    response: CodeApprovalResponse,
  ) => Promise<void>;
  rows: readonly CodeTimelineRow[];
}) {
  const endRef = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    if (rows.length === 0 && approvals.length === 0) return;
    endRef.current?.scrollIntoView({ block: "end" });
  }, [approvals.length, rows.length]);

  return (
    <div
      aria-busy={loading}
      aria-label="Code task timeline"
      className="min-h-0 flex-1 overflow-y-auto"
      data-testid="code-timeline"
      role="log"
    >
      <div className="mx-auto flex min-h-full w-full max-w-4xl flex-col gap-3 px-4 py-5">
        {loading && rows.length === 0 ? (
          <div className="flex flex-1 items-center justify-center gap-2 text-sm text-muted-foreground">
            <CircleDashed className="h-4 w-4 animate-spin motion-reduce:animate-none" />
            Opening task…
          </div>
        ) : rows.length === 0 && approvals.length === 0 ? (
          <div className="flex flex-1 flex-col items-center justify-center text-center">
            <Bot className="h-8 w-8 text-muted-foreground/40" />
            <p className="mt-3 text-sm font-medium">Start with a request</p>
            <p className="mt-1 max-w-sm text-xs text-muted-foreground">
              Ask Codex to inspect, change, or verify this project.
            </p>
          </div>
        ) : (
          <>
            {rows.map((row) => (
              <TimelineRow key={row.key} row={row} />
            ))}
            {approvals.map((approval) => (
              <CodeApprovalCard
                approval={approval}
                canRespond={canRespond(approval)}
                key={`${codeApprovalIdentityKey(approval)}:${approval.itemId}:${approval.sequence}`}
                onRespond={(response) => onRespond(approval, response)}
              />
            ))}
          </>
        )}
        <div aria-hidden="true" ref={endRef} />
      </div>
    </div>
  );
}
