import { Check, Minus, Plus } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import type {
  CodeGitCapability,
  CodeGitChangeFile,
  CodeGitReadyStatus,
} from "../api/codeGitTypes";

export function CodeGitChangesActions({
  busy,
  onCommit,
  onMutate,
  status,
}: {
  busy: boolean;
  onCommit: () => void;
  onMutate: (operation: "stage" | "unstage", file: CodeGitChangeFile) => void;
  status: CodeGitReadyStatus;
}) {
  const partialPaths = new Set(
    status.staged.files
      .filter((staged) =>
        status.unstaged.files.some((unstaged) => unstaged.path === staged.path),
      )
      .map((file) => file.path),
  );

  return (
    <div className="space-y-4 p-4" aria-busy={busy}>
      <div className="flex items-center justify-between gap-3">
        <div>
          <h3 className="text-balance text-sm font-semibold">
            Working changes
          </h3>
          <p className="text-pretty text-xs text-muted-foreground">
            Commit includes staged files only. Unstaged work is preserved.
          </p>
        </div>
        <Button
          disabled={busy || !status.capabilities.commit.enabled}
          onClick={onCommit}
          size="sm"
          title={status.capabilities.commit.reason ?? "Commit staged changes"}
        >
          <Check />
          Commit staged changes
        </Button>
      </div>
      <ChangeSection
        action="unstage"
        busy={busy}
        capability={status.capabilities.unstage}
        files={status.staged.files}
        label="Staged changes"
        onMutate={onMutate}
        partialPaths={partialPaths}
      />
      <ChangeSection
        action="stage"
        busy={busy}
        capability={status.capabilities.stage}
        files={status.unstaged.files}
        label="Unstaged changes"
        onMutate={onMutate}
        partialPaths={partialPaths}
      />
    </div>
  );
}

function ChangeSection({
  action,
  busy,
  capability,
  files,
  label,
  onMutate,
  partialPaths,
}: {
  action: "stage" | "unstage";
  busy: boolean;
  capability: CodeGitCapability;
  files: CodeGitChangeFile[];
  label: string;
  onMutate: (operation: "stage" | "unstage", file: CodeGitChangeFile) => void;
  partialPaths: Set<string>;
}) {
  const headingId = `code-git-${action}-heading`;
  return (
    <section aria-labelledby={headingId}>
      <div className="mb-2 flex items-center justify-between gap-2">
        <h4 className="text-xs font-semibold" id={headingId}>
          {label}
        </h4>
        <span className="tabular-nums text-2xs text-muted-foreground">
          {files.length} {files.length === 1 ? "file" : "files"}
        </span>
      </div>
      {files.length === 0 ? (
        <p className="rounded-md border border-dashed p-3 text-pretty text-xs text-muted-foreground">
          No {label.toLowerCase()}.
        </p>
      ) : (
        <ul aria-label={label} className="divide-y rounded-md border">
          {files.map((file) => {
            const disabled = busy || !capability.enabled;
            const actionLabel = `${action === "stage" ? "Stage" : "Unstage"} ${file.path}`;
            return (
              <li className="flex items-center gap-2 p-2" key={file.fileId}>
                <button
                  className="min-w-0 flex-1 rounded-sm text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                  title={file.path}
                  type="button"
                >
                  <span className="block truncate text-xs">{file.path}</span>
                  <span className="flex gap-2 text-2xs text-muted-foreground">
                    <span className="capitalize">{file.status}</span>
                    {partialPaths.has(file.path) ? (
                      <span>Partially staged</span>
                    ) : null}
                    {file.binary ? <span>Binary</span> : null}
                    <span className="tabular-nums">
                      +{file.additions} −{file.deletions}
                    </span>
                  </span>
                </button>
                <Button
                  aria-label={actionLabel}
                  disabled={disabled}
                  onClick={() => onMutate(action, file)}
                  size="icon-xs"
                  title={actionLabel}
                  variant="ghost"
                >
                  {action === "stage" ? <Plus /> : <Minus />}
                </Button>
                {!capability.enabled && capability.reason ? (
                  <span className={cn("sr-only")}>{capability.reason}</span>
                ) : null}
              </li>
            );
          })}
        </ul>
      )}
      {!capability.enabled && capability.reason ? (
        <p className="mt-1 text-pretty text-2xs text-muted-foreground">
          {capability.reason}
        </p>
      ) : null}
    </section>
  );
}
