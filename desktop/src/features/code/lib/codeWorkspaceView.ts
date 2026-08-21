import type {
  CodeBoundThreadSummary,
  CodeRuntimePhase,
  CodeThreadLifecycleState,
  CodeThreadPreparation,
} from "../api/types";

export type CodeRuntimePresentation = {
  label: string;
  description: string;
  tone: "neutral" | "pending" | "ready" | "error";
};

export type CodeThreadLifecycleCapabilities = {
  canArchive: boolean;
  canExecute: boolean;
  canFork: boolean;
  canReadChanges: boolean;
  canRename: boolean;
  canUnarchive: boolean;
  stable: boolean;
};

/** UI affordances projected from native lifecycle authority. */
export function codeThreadLifecycleCapabilities(
  lifecycle: CodeThreadLifecycleState,
): CodeThreadLifecycleCapabilities {
  switch (lifecycle) {
    case "active":
      return {
        canArchive: true,
        canExecute: true,
        canFork: true,
        canReadChanges: true,
        canRename: true,
        canUnarchive: false,
        stable: true,
      };
    case "archived":
      return {
        canArchive: false,
        canExecute: false,
        canFork: false,
        canReadChanges: true,
        canRename: true,
        canUnarchive: true,
        stable: true,
      };
    default:
      return {
        canArchive: false,
        canExecute: false,
        canFork: false,
        canReadChanges: false,
        canRename: false,
        canUnarchive: false,
        stable: false,
      };
  }
}

/** Concise row/header label for one authoritative lifecycle state. */
export function codeThreadLifecycleLabel(
  lifecycle: CodeThreadLifecycleState,
): string {
  switch (lifecycle) {
    case "active":
      return "Active";
    case "archiving":
      return "Archiving…";
    case "archived":
      return "Archived";
    case "unarchiving":
      return "Restoring…";
    case "unknown":
      return "Status unknown";
  }
}

/** Human-facing copy for native-owned runtime phases. */
export function codeRuntimePresentation(
  phase: CodeRuntimePhase | null,
): CodeRuntimePresentation {
  switch (phase) {
    case "ready":
      return {
        label: "Ready",
        description: "Codex is ready for this project.",
        tone: "ready",
      };
    case "starting":
      return {
        label: "Starting",
        description: "Starting the local Codex runtime…",
        tone: "pending",
      };
    case "initializing":
      return {
        label: "Initializing",
        description: "Establishing the Codex app-server session…",
        tone: "pending",
      };
    case "stopping":
      return {
        label: "Stopping",
        description: "Stopping the local Codex runtime…",
        tone: "pending",
      };
    case "notInstalled":
      return {
        label: "Codex not found",
        description: "Install the supported Codex CLI, then check again.",
        tone: "error",
      };
    case "failed":
      return {
        label: "Runtime unavailable",
        description:
          "Codex could not initialize. Check compatibility and retry.",
        tone: "error",
      };
    case "stopped":
      return {
        label: "Stopped",
        description: "Codex will start when this workspace opens.",
        tone: "neutral",
      };
    default:
      return {
        label: "Checking runtime",
        description: "Checking the local Codex installation…",
        tone: "pending",
      };
  }
}

export function codeThreadLabel(thread: CodeBoundThreadSummary): string {
  return (
    thread.thread?.name?.trim() ||
    thread.thread?.preview?.trim() ||
    `Task ${thread.binding.codexThreadId.slice(0, 8)}`
  );
}

/** Stable action copy for every durable start/fork preparation state. */
export function codeThreadPreparationLabels(
  preparation: Pick<CodeThreadPreparation, "operation" | "state">,
): { action: string; title: string } {
  if (preparation.operation === "fork") {
    return preparation.state === "starting"
      ? { action: "Recover fork", title: "Fork needs recovery" }
      : { action: "Continue fork", title: "Prepared fork" };
  }
  return preparation.state === "starting"
    ? { action: "Recover task", title: "Needs recovery" }
    : { action: "Start task", title: "Prepared task" };
}

/** Match a local task search without changing the native scoped thread list. */
export function codeThreadMatchesSearch(
  thread: CodeBoundThreadSummary,
  query: string,
): boolean {
  const normalizedQuery = query.trim().toLowerCase();
  if (normalizedQuery.length === 0) return true;

  const threadId = thread.binding.codexThreadId;
  return [
    thread.thread?.name,
    thread.thread?.preview,
    threadId,
    threadId.slice(0, 8),
  ].some((candidate) => candidate?.toLowerCase().includes(normalizedQuery));
}

/** Preserve a routed selection when valid, otherwise choose the newest row. */
export function selectCodeThreadId(
  requestedThreadId: string | null,
  threads: readonly CodeBoundThreadSummary[],
): string | null {
  if (
    requestedThreadId &&
    threads.some((thread) => thread.binding.codexThreadId === requestedThreadId)
  ) {
    return requestedThreadId;
  }
  return threads[0]?.binding.codexThreadId ?? null;
}

/** Native ordering is authoritative; group unfinished claims by state only. */
export function groupCodePreparations(
  preparations: readonly CodeThreadPreparation[],
): {
  starting: CodeThreadPreparation[];
  prepared: CodeThreadPreparation[];
} {
  return {
    starting: preparations.filter(({ state }) => state === "starting"),
    prepared: preparations.filter(({ state }) => state === "prepared"),
  };
}
