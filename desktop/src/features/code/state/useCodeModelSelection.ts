import { useQuery, useQueryClient } from "@tanstack/react-query";
import * as React from "react";

import { codeWorkspaceApi } from "../api/codeWorkspace";
import type {
  CodeBoundThreadOpenResult,
  CodeModelSelection,
  CodeModelsCatalog,
} from "../api/types";
import {
  type CodeModelChoice,
  codeModelSelectionFromFreshOpen,
  codeModelSelectionFromOpen,
  defaultCodeModelSelection,
  selectCodeModel,
  selectCodeReasoningEffort,
} from "../lib/codeModelSelection";
import {
  codeModelsQueryOptions,
  codeSessionQueryKeys,
} from "./codeSessionQueries";

type FailedSelection = {
  contextKey: string;
  selection: CodeModelSelection;
};

export type CodeModelSelectionController = {
  catalog: CodeModelsCatalog | null;
  choice: CodeModelChoice | null;
  turnSelection: CodeModelSelection | null;
  newThreadSelection: CodeModelSelection | null;
  loading: boolean;
  saving: boolean;
  error: string | null;
  chooseModel(model: string): void;
  chooseReasoningEffort(reasoningEffort: string): void;
  revalidateCatalog(): void;
  retry(): void;
  seedOpenedThread(
    opened: CodeBoundThreadOpenResult,
    pendingSelection?: CodeModelSelection | null,
  ): void;
};

function selectionContextKey(
  runtimeGeneration: number | null,
  selectedThreadId: string | null,
): string | null {
  if (runtimeGeneration === null) return null;
  return `${runtimeGeneration}:${selectedThreadId ?? "new"}`;
}

function choicesEqual(
  left: CodeModelChoice | undefined,
  right: CodeModelChoice,
): boolean {
  return (
    left?.model === right.model &&
    left.reasoningEffort === right.reasoningEffort
  );
}

/** Own generation-safe catalog, per-thread choice, and recent persistence. */
export function useCodeModelSelection({
  openedThread,
  runtimeGeneration,
  runtimeReady,
  selectedThreadId,
}: {
  openedThread: CodeBoundThreadOpenResult | null;
  runtimeGeneration: number | null;
  runtimeReady: boolean;
  selectedThreadId: string | null;
}): CodeModelSelectionController {
  const queryClient = useQueryClient();
  const queryGeneration = runtimeGeneration ?? 0;
  const queryEnabled = runtimeReady && runtimeGeneration !== null;
  const catalogQuery = useQuery({
    ...codeModelsQueryOptions(queryGeneration),
    enabled: queryEnabled,
  });
  const catalog =
    queryEnabled && !catalogQuery.isError && !catalogQuery.isFetching
      ? (catalogQuery.data ?? null)
      : null;
  const contextKey = selectionContextKey(runtimeGeneration, selectedThreadId);
  const [choices, setChoices] = React.useState<
    ReadonlyMap<string, CodeModelChoice>
  >(() => new Map());
  const [pendingContextKey, setPendingContextKey] = React.useState<
    string | null
  >(null);
  const [failedSelection, setFailedSelection] =
    React.useState<FailedSelection | null>(null);
  const mutationSequenceRef = React.useRef(0);
  const pendingMutationRef = React.useRef<number | null>(null);
  const generationRef = React.useRef(runtimeGeneration);
  const previousGenerationRef = React.useRef(runtimeGeneration);
  generationRef.current = runtimeGeneration;

  React.useEffect(() => {
    if (previousGenerationRef.current === runtimeGeneration) return;
    previousGenerationRef.current = runtimeGeneration;
    mutationSequenceRef.current += 1;
    pendingMutationRef.current = null;
    setPendingContextKey(null);
    setFailedSelection(null);
    setChoices(new Map());
  }, [runtimeGeneration]);

  const defaultSelection = React.useMemo(
    () => (catalog ? defaultCodeModelSelection(catalog) : null),
    [catalog],
  );
  const fallbackChoice = React.useMemo<CodeModelChoice | null>(
    () =>
      selectedThreadId
        ? openedThread
          ? {
              model: openedThread.model,
              reasoningEffort: openedThread.reasoningEffort,
            }
          : null
        : defaultSelection,
    [defaultSelection, openedThread, selectedThreadId],
  );
  const storedChoice =
    contextKey === null ? undefined : choices.get(contextKey);
  const storedChoiceIsAvailable =
    storedChoice !== undefined &&
    codeModelSelectionFromOpen(catalog, storedChoice).turnSelection !== null;
  const choice =
    contextKey === null
      ? null
      : selectedThreadId === null && !storedChoiceIsAvailable
        ? fallbackChoice
        : (storedChoice ?? fallbackChoice);
  const resolved = React.useMemo(
    () => (choice ? codeModelSelectionFromOpen(catalog, choice) : null),
    [catalog, choice],
  );

  const persistSelection = React.useCallback(
    async (selection: CodeModelSelection, targetContextKey: string) => {
      if (
        catalog === null ||
        runtimeGeneration === null ||
        pendingMutationRef.current !== null
      ) {
        return;
      }
      const model = catalog.models.find(
        (option) => option.model === selection.model,
      );
      if (
        model === undefined ||
        !model.supportedReasoningEfforts.some(
          (option) => option.reasoningEffort === selection.reasoningEffort,
        )
      ) {
        return;
      }

      const previous = choices.get(targetContextKey);
      const mutationSequence = ++mutationSequenceRef.current;
      pendingMutationRef.current = mutationSequence;
      setPendingContextKey(targetContextKey);
      setFailedSelection(null);
      setChoices((current) => {
        const next = new Map(current);
        next.set(targetContextKey, selection);
        return next;
      });
      try {
        const persisted =
          await codeWorkspaceApi.setCodeModelSelection(selection);
        if (
          pendingMutationRef.current !== mutationSequence ||
          generationRef.current !== runtimeGeneration
        ) {
          return;
        }
        setChoices((current) => {
          const next = new Map(current);
          next.set(targetContextKey, persisted);
          return next;
        });
        queryClient.setQueryData<CodeModelsCatalog>(
          codeSessionQueryKeys.models(runtimeGeneration),
          (current) =>
            current?.runtimeGeneration === runtimeGeneration
              ? { ...current, recentSelection: persisted }
              : current,
        );
      } catch {
        if (
          pendingMutationRef.current !== mutationSequence ||
          generationRef.current !== runtimeGeneration
        ) {
          return;
        }
        setChoices((current) => {
          if (!choicesEqual(current.get(targetContextKey), selection)) {
            return current;
          }
          const next = new Map(current);
          if (previous === undefined) next.delete(targetContextKey);
          else next.set(targetContextKey, previous);
          return next;
        });
        setFailedSelection({ contextKey: targetContextKey, selection });
      } finally {
        if (pendingMutationRef.current === mutationSequence) {
          pendingMutationRef.current = null;
          setPendingContextKey(null);
        }
      }
    },
    [catalog, choices, queryClient, runtimeGeneration],
  );

  const chooseModel = React.useCallback(
    (model: string) => {
      if (catalog === null || contextKey === null) return;
      const selection = selectCodeModel(catalog, choice, model);
      if (selection !== null) {
        void persistSelection(selection, contextKey);
      }
    },
    [catalog, choice, contextKey, persistSelection],
  );

  const chooseReasoningEffort = React.useCallback(
    (reasoningEffort: string) => {
      if (catalog === null || contextKey === null) return;
      const selection = selectCodeReasoningEffort(
        catalog,
        choice,
        reasoningEffort,
      );
      if (selection !== null) {
        void persistSelection(selection, contextKey);
      }
    },
    [catalog, choice, contextKey, persistSelection],
  );

  const retry = React.useCallback(() => {
    if (failedSelection !== null && failedSelection.contextKey === contextKey) {
      void persistSelection(failedSelection.selection, contextKey);
      return;
    }
    void catalogQuery.refetch();
  }, [catalogQuery.refetch, contextKey, failedSelection, persistSelection]);

  const revalidateCatalog = React.useCallback(() => {
    if (runtimeGeneration === null) return;
    void queryClient.invalidateQueries({
      exact: true,
      queryKey: codeSessionQueryKeys.models(runtimeGeneration),
    });
  }, [queryClient, runtimeGeneration]);

  const seedOpenedThread = React.useCallback(
    (
      opened: CodeBoundThreadOpenResult,
      pendingSelection: CodeModelSelection | null = null,
    ) => {
      if (runtimeGeneration === null) return;
      const targetContextKey = selectionContextKey(
        runtimeGeneration,
        opened.thread.id,
      );
      if (targetContextKey === null) return;
      const resolvedOpen = codeModelSelectionFromFreshOpen(
        catalog,
        {
          model: opened.model,
          reasoningEffort: opened.reasoningEffort,
        },
        pendingSelection,
      );
      setChoices((current) => {
        const next = new Map(current);
        next.set(targetContextKey, resolvedOpen.choice);
        return next;
      });
    },
    [catalog, runtimeGeneration],
  );

  const activePersistenceFailed =
    failedSelection !== null && failedSelection.contextKey === contextKey;
  const catalogFailed =
    queryEnabled && catalog === null && catalogQuery.isError;
  return {
    catalog,
    choice,
    turnSelection: catalog === null ? null : (resolved?.turnSelection ?? null),
    newThreadSelection:
      selectedThreadId === null
        ? (resolved?.turnSelection ?? null)
        : defaultSelection,
    loading: queryEnabled && catalog === null && catalogQuery.isFetching,
    saving: pendingContextKey !== null,
    error: activePersistenceFailed
      ? "Model selection wasn’t saved. The previous choice was restored."
      : catalogFailed
        ? "Model options are unavailable. Codex defaults will be used."
        : null,
    chooseModel,
    chooseReasoningEffort,
    revalidateCatalog,
    retry,
    seedOpenedThread,
  };
}
