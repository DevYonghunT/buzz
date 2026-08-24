import { MessageSquarePlus } from "lucide-react";
import * as React from "react";

import type { ProjectPullRequestCommentAnchor } from "@/features/projects/hooks";
import type { ProjectPullRequestComment } from "@/features/projects/projectPullRequests.mjs";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ProjectRepoDiffFile } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { ProjectPullRequestInlineCommentThread } from "./ProjectPullRequestInlineComments";

type DiffRow = {
  content: string;
  key: string;
  newLine: number | null;
  oldLine: number | null;
  type: "add" | "context" | "delete" | "hunk";
};

export type InlineCommentControls = {
  activeAnchor: ProjectPullRequestCommentAnchor | null;
  canRequestChanges: boolean;
  comments: ProjectPullRequestComment[];
  isSending: boolean;
  onCancel: () => void;
  onStart: (anchor: ProjectPullRequestCommentAnchor) => void;
  onSubmit: (
    anchor: ProjectPullRequestCommentAnchor,
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    decision?: "request-changes",
  ) => Promise<unknown>;
  profiles?: UserProfileLookup;
};

function parseHunkHeader(line: string) {
  const match = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
  if (!match) return null;
  return { oldLine: Number(match[1]), newLine: Number(match[2]) };
}

function diffRows(file: ProjectRepoDiffFile): DiffRow[] {
  let oldLine = 0;
  let newLine = 0;
  return file.patch
    .trimEnd()
    .split("\n")
    .filter(
      (line) =>
        !line.startsWith("diff --git ") &&
        !line.startsWith("index ") &&
        !line.startsWith("--- ") &&
        !line.startsWith("+++ "),
    )
    .map((line, index) => {
      const hunk = parseHunkHeader(line);
      let row: Omit<DiffRow, "key">;
      if (hunk) {
        oldLine = hunk.oldLine;
        newLine = hunk.newLine;
        row = { content: line, oldLine: null, newLine: null, type: "hunk" };
      } else if (line.startsWith("+")) {
        row = {
          content: line.slice(1),
          oldLine: null,
          newLine: newLine++,
          type: "add",
        };
      } else if (line.startsWith("-")) {
        row = {
          content: line.slice(1),
          oldLine: oldLine++,
          newLine: null,
          type: "delete",
        };
      } else {
        row = {
          content: line.startsWith(" ") ? line.slice(1) : line,
          oldLine: oldLine++,
          newLine: newLine++,
          type: "context",
        };
      }
      // Rows are computed once per patch in source order, so a positional
      // index is a stable, cheap key.
      return { ...row, key: `${file.path}:${index}` };
    });
}

function diffLineClassName(type: DiffRow["type"]) {
  if (type === "add") return "border-green-500/10 border-l-2 bg-green-500/10";
  if (type === "delete")
    return "border-destructive/10 border-l-2 bg-destructive/10";
  if (type === "hunk") return "bg-sky-500/10 text-sky-500";
  return "border-transparent border-l-2";
}

function linePrefix(type: DiffRow["type"]) {
  if (type === "add") return "+";
  if (type === "delete") return "-";
  return " ";
}

function commentAnchorForRow(
  file: ProjectRepoDiffFile,
  row: DiffRow,
): ProjectPullRequestCommentAnchor | null {
  if (row.type === "hunk") return null;
  const side = row.type === "delete" ? "old" : "new";
  const line = side === "old" ? row.oldLine : row.newLine;
  return line ? { line, path: file.path, side } : null;
}

function anchorsEqual(
  left: ProjectPullRequestCommentAnchor | null,
  right: ProjectPullRequestCommentAnchor | null,
) {
  return Boolean(
    left &&
      right &&
      left.line === right.line &&
      left.path === right.path &&
      left.side === right.side,
  );
}

export function DiffPreview({
  file,
  focusedAnchor,
  inlineComments,
}: {
  file: ProjectRepoDiffFile;
  focusedAnchor?: ProjectPullRequestCommentAnchor | null;
  inlineComments?: InlineCommentControls;
}) {
  const rows = diffRows(file);
  const focusedRowRef = React.useRef<HTMLDivElement | null>(null);
  const [highlightedAnchor, setHighlightedAnchor] =
    React.useState<ProjectPullRequestCommentAnchor | null>(null);

  React.useEffect(() => {
    if (!focusedAnchor || focusedAnchor.path !== file.path) {
      setHighlightedAnchor(null);
      return;
    }

    setHighlightedAnchor(focusedAnchor);
    let isListeningForInteraction = false;
    const clearHighlight = () => setHighlightedAnchor(null);
    const clearHighlightOnKeyDown = (event: KeyboardEvent) => {
      if (["Alt", "Control", "Meta", "Shift"].includes(event.key)) return;
      clearHighlight();
    };
    const frame = window.requestAnimationFrame(() => {
      focusedRowRef.current?.scrollIntoView({
        behavior: "smooth",
        block: "center",
      });
      focusedRowRef.current?.focus({ preventScroll: true });
      window.addEventListener("pointerdown", clearHighlight, true);
      window.addEventListener("keydown", clearHighlightOnKeyDown, true);
      isListeningForInteraction = true;
    });
    return () => {
      window.cancelAnimationFrame(frame);
      if (isListeningForInteraction) {
        window.removeEventListener("pointerdown", clearHighlight, true);
        window.removeEventListener("keydown", clearHighlightOnKeyDown, true);
      }
    };
  }, [file.path, focusedAnchor]);

  if (file.binary) {
    return (
      <div className="bg-muted/20 px-4 py-4 text-sm text-muted-foreground">
        Binary file preview is not available.
      </div>
    );
  }

  if (rows.length === 0) {
    return (
      <div className="bg-muted/20 px-4 py-4 text-sm text-muted-foreground">
        No textual diff is available for this file.
      </div>
    );
  }

  return (
    <div className="overflow-x-auto bg-background/70 font-mono text-xs leading-5">
      {file.truncated ? (
        <div className="border-border/40 border-b bg-amber-500/10 px-4 py-2 text-amber-600 dark:text-amber-400">
          Large diff truncated — showing the first {rows.length} lines. Use a
          local checkout to review the full change.
        </div>
      ) : null}
      {rows.map((row) => {
        const anchor = commentAnchorForRow(file, row);
        const comments =
          anchor && inlineComments
            ? inlineComments.comments.filter(
                (comment) =>
                  comment.anchor && anchorsEqual(comment.anchor, anchor),
              )
            : [];
        const isActive = anchorsEqual(
          inlineComments?.activeAnchor ?? null,
          anchor,
        );
        const isFocused = anchorsEqual(highlightedAnchor, anchor);
        return (
          <React.Fragment key={row.key}>
            <div
              className={cn(
                "group grid min-h-5 grid-cols-[3rem_3rem_2rem_1.5rem_minmax(0,1fr)]",
                diffLineClassName(row.type),
                isFocused && "bg-primary/10 ring-1 ring-primary/40 ring-inset",
              )}
              data-line={anchor?.line}
              data-path={anchor?.path}
              data-side={anchor?.side}
              data-testid={
                isFocused
                  ? "project-diff-focused-line"
                  : anchor
                    ? "project-diff-line"
                    : undefined
              }
              ref={isFocused ? focusedRowRef : undefined}
              tabIndex={isFocused ? -1 : undefined}
            >
              <span className="select-none border-border/40 border-r px-2 text-right text-muted-foreground/70">
                {row.oldLine ?? " "}
              </span>
              <span className="select-none border-border/40 border-r px-2 text-right text-muted-foreground/70">
                {row.newLine ?? " "}
              </span>
              <span className="flex select-none items-center justify-center">
                {anchor && inlineComments ? (
                  <button
                    aria-label={`Comment on ${anchor.path} ${anchor.side} line ${anchor.line}`}
                    className={cn(
                      "flex h-5 w-5 items-center justify-center rounded text-muted-foreground opacity-0 hover:bg-primary hover:text-primary-foreground focus-visible:opacity-100 focus-visible:outline-hidden group-hover:opacity-100",
                      (comments.length > 0 || isActive) && "opacity-100",
                    )}
                    data-testid="project-diff-add-comment"
                    onClick={() => inlineComments.onStart(anchor)}
                    title="Add line comment"
                    type="button"
                  >
                    <MessageSquarePlus className="h-3.5 w-3.5" />
                  </button>
                ) : null}
              </span>
              <span
                className={cn(
                  "select-none px-2",
                  row.type === "add" && "text-green-500",
                  row.type === "delete" && "text-destructive",
                )}
              >
                {linePrefix(row.type)}
              </span>
              <code className="min-w-0 whitespace-pre pr-3 text-foreground">
                {row.content || " "}
              </code>
            </div>
            {anchor && inlineComments ? (
              <ProjectPullRequestInlineCommentThread
                activeAnchor={isActive ? anchor : null}
                canRequestChanges={inlineComments.canRequestChanges}
                comments={comments}
                isSending={inlineComments.isSending}
                onCancel={inlineComments.onCancel}
                onSubmit={(content, mentionPubkeys, mediaTags, decision) =>
                  inlineComments.onSubmit(
                    anchor,
                    content,
                    mentionPubkeys,
                    mediaTags,
                    decision,
                  )
                }
                profiles={inlineComments.profiles}
              />
            ) : null}
          </React.Fragment>
        );
      })}
    </div>
  );
}
