import { useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import type { CodeThreadBindingScope, CodeWorkspaceEvent } from "../api/types";
import { codeSessionQueryKeys } from "./codeSessionQueries";

/** Invalidate a selected diff once for each new diff-producing event. */
export function useCodeChangesInvalidation({
  enabled,
  runtimeGeneration,
  scope,
  selectedThreadEvents,
  selectedThreadId,
}: {
  enabled: boolean;
  runtimeGeneration: number | null;
  scope: CodeThreadBindingScope;
  selectedThreadEvents: readonly CodeWorkspaceEvent[];
  selectedThreadId: string | null;
}) {
  const queryClient = useQueryClient();
  const eventIdentity = React.useMemo(() => {
    if (selectedThreadId === null || runtimeGeneration === null) return null;
    const latestSequence = selectedThreadEvents.reduce(
      (latest, event) =>
        event.runtimeGeneration === runtimeGeneration &&
        (event.kind === "turn/diff/updated" ||
          event.kind === "item/fileChange/patchUpdated" ||
          event.kind === "turn/completed")
          ? Math.max(latest, event.sequence)
          : latest,
      0,
    );
    return latestSequence === 0
      ? null
      : JSON.stringify([selectedThreadId, runtimeGeneration, latestSequence]);
  }, [runtimeGeneration, selectedThreadEvents, selectedThreadId]);
  const queryIdentity =
    selectedThreadId === null || runtimeGeneration === null
      ? null
      : JSON.stringify([selectedThreadId, runtimeGeneration]);
  const previousRef = React.useRef<{
    queryIdentity: string;
    eventIdentity: string | null;
  } | null>(null);

  React.useEffect(() => {
    if (
      !enabled ||
      selectedThreadId === null ||
      runtimeGeneration === null ||
      queryIdentity === null
    ) {
      previousRef.current = null;
      return;
    }
    const previous = previousRef.current;
    if (previous === null || previous.queryIdentity !== queryIdentity) {
      previousRef.current = { queryIdentity, eventIdentity };
      return;
    }
    if (eventIdentity === null || previous.eventIdentity === eventIdentity) {
      return;
    }
    previousRef.current = { queryIdentity, eventIdentity };
    void queryClient.invalidateQueries({
      exact: true,
      queryKey: codeSessionQueryKeys.threadChanges({
        scope,
        threadId: selectedThreadId,
        runtimeGeneration,
      }),
    });
    void queryClient.invalidateQueries({
      exact: true,
      queryKey: codeSessionQueryKeys.threadGitStatus({
        scope,
        threadId: selectedThreadId,
        runtimeGeneration,
      }),
    });
  }, [
    enabled,
    eventIdentity,
    queryClient,
    queryIdentity,
    runtimeGeneration,
    scope,
    selectedThreadId,
  ]);
}
