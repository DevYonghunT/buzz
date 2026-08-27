import * as React from "react";

import {
  CODE_CHANGES_PANEL_MAX_WIDTH_PX,
  CODE_CHANGES_PANEL_MIN_WIDTH_PX,
  CODE_TASKS_PANEL_MAX_WIDTH_PX,
  CODE_TASKS_PANEL_MIN_WIDTH_PX,
  type CodePanelWidths,
  readCodePanelWidths,
  resolveCodePanelLayout,
  writeCodePanelWidths,
} from "../lib/codePanelLayout";

function localStorageOrNull(): Storage | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage;
  } catch {
    return null;
  }
}

function initialContainerWidth(): number {
  return typeof window === "undefined" ? 0 : window.innerWidth;
}

/** Measured, device-persisted widths for the three-pane Code workspace. */
export function useCodePanelLayout({
  changesOpen,
  tasksOpen,
}: {
  changesOpen: boolean;
  tasksOpen: boolean;
}) {
  const containerRef = React.useRef<HTMLDivElement>(null);
  const [containerWidth, setContainerWidth] = React.useState(
    initialContainerWidth,
  );
  const [preferred, setPreferred] = React.useState<CodePanelWidths>(() =>
    readCodePanelWidths(localStorageOrNull()),
  );

  React.useLayoutEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const update = () => {
      const next = Math.round(container.getBoundingClientRect().width);
      setContainerWidth((current) => (current === next ? current : next));
    };
    update();
    if (typeof ResizeObserver === "undefined") {
      window.addEventListener("resize", update);
      return () => window.removeEventListener("resize", update);
    }
    const observer = new ResizeObserver(update);
    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  React.useEffect(() => {
    try {
      writeCodePanelWidths(localStorageOrNull(), preferred);
    } catch {
      // Storage is optional; the in-memory layout remains fully functional.
    }
  }, [preferred]);

  const layout = resolveCodePanelLayout({
    changesOpen,
    containerWidth,
    preferred,
    tasksOpen,
  });
  const setTasksWidth = React.useCallback((width: number) => {
    setPreferred((current) => ({
      ...current,
      tasks: Math.min(
        CODE_TASKS_PANEL_MAX_WIDTH_PX,
        Math.max(CODE_TASKS_PANEL_MIN_WIDTH_PX, Math.round(width)),
      ),
    }));
  }, []);
  const setChangesWidth = React.useCallback((width: number) => {
    setPreferred((current) => ({
      ...current,
      changes: Math.min(
        CODE_CHANGES_PANEL_MAX_WIDTH_PX,
        Math.max(CODE_CHANGES_PANEL_MIN_WIDTH_PX, Math.round(width)),
      ),
    }));
  }, []);

  return {
    ...layout,
    containerRef,
    setChangesWidth,
    setTasksWidth,
  };
}
