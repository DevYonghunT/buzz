import type {
  CodeBoundThreadSummary,
  CodeThreadBindingScope,
  CodeWorkspaceEvent,
} from "../api/types";
import { codeThreadLifecycleCapabilities } from "../lib/codeWorkspaceView";
import { useCodeChangesInvalidation } from "./useCodeChangesInvalidation";
import { useCodeGitHandoff } from "./useCodeGitHandoff";

/** Own selected-thread Git handoff state above the optional Changes inspector. */
export function useCodeWorkspaceGitHandoff({
  interactionReady,
  lifecycleBlocked,
  runtimeGeneration,
  runtimeReady,
  scope,
  selectedRow,
  selectedThreadEvents,
}: {
  interactionReady: boolean;
  lifecycleBlocked: boolean;
  runtimeGeneration: number | null;
  runtimeReady: boolean;
  scope: CodeThreadBindingScope;
  selectedRow: CodeBoundThreadSummary | null;
  selectedThreadEvents: readonly CodeWorkspaceEvent[];
}) {
  const changesEnabled =
    selectedRow !== null &&
    codeThreadLifecycleCapabilities(selectedRow.lifecycle).canReadChanges &&
    !lifecycleBlocked &&
    (selectedRow.lifecycle === "archived" ? runtimeReady : interactionReady);

  useCodeChangesInvalidation({
    enabled: interactionReady,
    runtimeGeneration,
    scope,
    selectedThreadEvents,
    selectedThreadId: selectedRow?.binding.codexThreadId ?? null,
  });

  const controller = useCodeGitHandoff({
    enabled:
      changesEnabled && selectedRow?.binding.executionMode === "worktree",
    runtimeGeneration,
    scope,
    threadId: selectedRow?.binding.codexThreadId ?? null,
  });

  return { changesEnabled, controller };
}
