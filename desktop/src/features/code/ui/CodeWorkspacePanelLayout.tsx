import type * as React from "react";

import { useCodePanelLayout } from "../state/useCodePanelLayout";
import { CodePanelResizeHandle } from "./CodePanelResizeHandle";

/** Three-pane shell with direct, accessible resizing and responsive overlay. */
export function CodeWorkspacePanelLayout({
  changes,
  conversation,
  tasks,
}: {
  changes: React.ReactNode;
  conversation: React.ReactNode;
  tasks: React.ReactNode;
}) {
  const panelLayout = useCodePanelLayout({
    changesOpen: changes !== null,
    tasksOpen: tasks !== null,
  });

  return (
    <div
      className="relative flex min-h-0 flex-1 overflow-hidden"
      ref={panelLayout.containerRef}
    >
      {tasks !== null ? (
        <div
          className="relative flex min-h-0 shrink-0"
          style={{ width: panelLayout.tasks.width }}
        >
          {tasks}
          {panelLayout.tasks.max > panelLayout.tasks.min ? (
            <CodePanelResizeHandle
              ariaLabel="Resize Tasks and conversation"
              className="absolute inset-y-0 right-0 translate-x-1/2"
              growDirection={1}
              max={panelLayout.tasks.max}
              min={panelLayout.tasks.min}
              onChange={panelLayout.setTasksWidth}
              testId="code-tasks-resize-handle"
              value={panelLayout.tasks.width}
            />
          ) : null}
        </div>
      ) : null}
      {conversation}
      {changes !== null ? (
        <div
          className={
            panelLayout.inspectorDocked
              ? "relative flex min-h-0 shrink-0"
              : "absolute inset-y-0 right-0 z-20 flex min-h-0 max-w-[calc(100%-3rem)] shadow-xl"
          }
          data-testid="code-changes-panel-frame"
          style={{ width: panelLayout.changes.width }}
        >
          {panelLayout.changes.max > panelLayout.changes.min ? (
            <CodePanelResizeHandle
              ariaLabel="Resize conversation and Changes"
              className="absolute inset-y-0 left-0 -translate-x-1/2"
              growDirection={-1}
              max={panelLayout.changes.max}
              min={panelLayout.changes.min}
              onChange={panelLayout.setChangesWidth}
              testId="code-changes-resize-handle"
              value={panelLayout.changes.width}
            />
          ) : null}
          {changes}
        </div>
      ) : null}
    </div>
  );
}
