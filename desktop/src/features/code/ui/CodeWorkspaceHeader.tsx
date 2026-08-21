import {
  GitBranch,
  LoaderCircle,
  PanelLeftClose,
  PanelLeftOpen,
  PanelRightClose,
  PanelRightOpen,
  SquareTerminal,
} from "lucide-react";

import type { CodeBoundThreadSummary } from "../api/types";
import { codeThreadLifecycleLabel } from "../lib/codeWorkspaceView";
import type { CodeModelSelectionController } from "../state/useCodeModelSelection";
import { CodeModelSelector } from "./CodeModelSelector";
import { getPlatformKeysById } from "@/shared/lib/keyboard-shortcuts";
import { Button } from "@/shared/ui/button";

export function CodeWorkspaceHeader({
  canReadChanges,
  inspectorOpen,
  modelSelection,
  modelSelectionDisabled,
  onInspectorOpenChange,
  onRetry,
  onSidebarOpenChange,
  onTerminalToggle,
  selectedLabel,
  selectedRow,
  selectedThreadId,
  selectionLoading,
  showRetry,
  sidebarOpen,
  terminalOpen,
  terminalVisible,
}: {
  canReadChanges: boolean;
  inspectorOpen: boolean;
  modelSelection: CodeModelSelectionController;
  modelSelectionDisabled: boolean;
  onInspectorOpenChange: (open: boolean) => void;
  onRetry: () => void;
  onSidebarOpenChange: (open: boolean) => void;
  onTerminalToggle: () => void;
  selectedLabel: string;
  selectedRow: CodeBoundThreadSummary | null;
  selectedThreadId: string | null;
  selectionLoading: boolean;
  showRetry: boolean;
  sidebarOpen: boolean;
  terminalOpen: boolean;
  terminalVisible: boolean;
}) {
  return (
    <header className="flex min-h-11 shrink-0 flex-wrap items-center gap-2 border-border/60 border-b px-3 py-1">
      <Button
        aria-label={sidebarOpen ? "Hide task sidebar" : "Show task sidebar"}
        className="h-7 w-7 shrink-0"
        onClick={() => onSidebarOpenChange(!sidebarOpen)}
        size="icon-xs"
        title={sidebarOpen ? "Hide task sidebar" : "Show task sidebar"}
        variant="ghost"
      >
        {sidebarOpen ? <PanelLeftClose /> : <PanelLeftOpen />}
      </Button>
      <div className="min-w-0 basis-48 flex-1">
        <h2 className="truncate text-sm font-semibold">
          {selectedThreadId ? selectedLabel : "Choose or create a task"}
        </h2>
        {selectedRow ? (
          <p className="flex min-w-0 items-center gap-1 text-2xs text-muted-foreground">
            <GitBranch className="h-3 w-3 shrink-0" />
            <span className="truncate">
              {selectedRow.binding.executionMode === "worktree"
                ? "Managed worktree"
                : "Local checkout"}
              {` · ${selectedRow.binding.baseRef.slice(0, 8)}`}
              {` · ${codeThreadLifecycleLabel(selectedRow.lifecycle)}`}
            </span>
          </p>
        ) : null}
      </div>
      <CodeModelSelector
        controller={modelSelection}
        disabled={modelSelectionDisabled}
      />
      {selectionLoading ? (
        <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <LoaderCircle className="h-3.5 w-3.5 animate-spin motion-reduce:animate-none" />
          Resuming…
        </span>
      ) : null}
      {showRetry ? (
        <Button onClick={onRetry} size="xs" variant="outline">
          Retry task
        </Button>
      ) : null}
      {terminalVisible ? (
        <Button
          aria-label={terminalOpen ? "Hide terminal" : "Show terminal"}
          aria-pressed={terminalOpen}
          className="size-7 shrink-0"
          data-testid="code-terminal-toggle"
          onClick={onTerminalToggle}
          size="icon-xs"
          title={`${terminalOpen ? "Hide" : "Show"} terminal (${getPlatformKeysById("toggle-code-terminal") ?? "Cmd/Ctrl+J"})`}
          variant="ghost"
        >
          <SquareTerminal />
        </Button>
      ) : null}
      {selectedRow && canReadChanges ? (
        <Button
          aria-label={
            inspectorOpen ? "Hide Changes inspector" : "Show Changes inspector"
          }
          className="h-7 w-7 shrink-0"
          onClick={() => onInspectorOpenChange(!inspectorOpen)}
          size="icon-xs"
          title={
            inspectorOpen ? "Hide Changes inspector" : "Show Changes inspector"
          }
          variant="ghost"
        >
          {inspectorOpen ? <PanelRightClose /> : <PanelRightOpen />}
        </Button>
      ) : null}
    </header>
  );
}
