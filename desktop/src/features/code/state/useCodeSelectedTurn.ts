import * as React from "react";

import type { CodeWorkspaceEvent } from "../api/types";
import {
  type CodeSessionState,
  selectCodeActiveTurns,
} from "./codeSessionReducer";

export type CodePendingTurn = {
  readonly runtimeGeneration: number;
  readonly turnId: string;
};

/** Reconcile optimistic turn starts with the selected and global event state. */
export function useCodeSelectedTurn({
  pendingTurns,
  runtimeReady,
  selectedThreadEvents,
  selectedThreadId,
  sessionState,
  setPendingTurns,
}: {
  pendingTurns: ReadonlyMap<string, CodePendingTurn>;
  runtimeReady: boolean;
  selectedThreadEvents: readonly CodeWorkspaceEvent[];
  selectedThreadId: string | null;
  sessionState: CodeSessionState;
  setPendingTurns: React.Dispatch<
    React.SetStateAction<ReadonlyMap<string, CodePendingTurn>>
  >;
}) {
  const activeTurns = selectedThreadId
    ? selectCodeActiveTurns(sessionState, selectedThreadId)
    : [];
  const activeTurn = activeTurns.at(-1) ?? null;
  const selectedPendingTurn = selectedThreadId
    ? (pendingTurns.get(selectedThreadId) ?? null)
    : null;
  const selectedPendingTurnObserved =
    selectedPendingTurn !== null &&
    selectedThreadEvents.some(
      (event) =>
        event.runtimeGeneration === selectedPendingTurn.runtimeGeneration &&
        event.turnId === selectedPendingTurn.turnId &&
        (event.kind === "turn/started" || event.kind === "turn/completed"),
    );
  const pendingTurn =
    selectedPendingTurn !== null &&
    !selectedPendingTurnObserved &&
    runtimeReady &&
    sessionState.runtimeGeneration === selectedPendingTurn.runtimeGeneration
      ? selectedPendingTurn
      : null;

  React.useEffect(() => {
    if (pendingTurns.size === 0) return;
    setPendingTurns((current) => {
      let next: Map<string, CodePendingTurn> | null = null;
      for (const [threadId, pending] of current) {
        const generationStillReady =
          sessionState.runtimeStatus?.phase === "ready" &&
          sessionState.runtimeGeneration === pending.runtimeGeneration;
        const observedActiveTurn = selectCodeActiveTurns(
          sessionState,
          threadId,
        ).some(
          (turn) =>
            turn.turnId === pending.turnId &&
            turn.runtimeGeneration === pending.runtimeGeneration,
        );
        const observedTurnEvent = sessionState.events.some(
          (event) =>
            event.runtimeGeneration === pending.runtimeGeneration &&
            event.threadId === threadId &&
            event.turnId === pending.turnId &&
            (event.kind === "turn/started" || event.kind === "turn/completed"),
        );
        if (!generationStillReady || observedActiveTurn || observedTurnEvent) {
          next ??= new Map(current);
          next.delete(threadId);
        }
      }
      return next ?? current;
    });
  }, [pendingTurns.size, sessionState, setPendingTurns]);

  return {
    activeTurn,
    effectiveTurnId: activeTurn?.turnId ?? pendingTurn?.turnId ?? null,
  };
}
