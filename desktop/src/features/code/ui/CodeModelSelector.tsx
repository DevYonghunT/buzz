import { ChevronDown, LoaderCircle } from "lucide-react";
import * as React from "react";

import {
  codeReasoningEffortLabel,
  findCodeModelOption,
} from "../lib/codeModelSelection";
import type { CodeModelSelectionController } from "../state/useCodeModelSelection";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

const MENU_WIDTH_CLASS = "w-80 max-w-[calc(100vw-2rem)] overflow-y-auto";
const MENU_MAX_HEIGHT =
  "min(20rem, var(--radix-dropdown-menu-content-available-height))";

function preventLockedMenuOpen(
  event: React.KeyboardEvent<HTMLButtonElement>,
  locked: boolean,
) {
  if (
    locked &&
    (event.key === "Enter" ||
      event.key === " " ||
      event.key === "ArrowDown" ||
      event.key === "ArrowUp")
  ) {
    event.preventDefault();
  }
}

export function CodeModelSelector({
  controller,
  disabled,
}: {
  controller: CodeModelSelectionController;
  disabled: boolean;
}) {
  const [modelMenuOpen, setModelMenuOpen] = React.useState(false);
  const [effortMenuOpen, setEffortMenuOpen] = React.useState(false);
  const selectedModel =
    controller.catalog && controller.choice
      ? findCodeModelOption(controller.catalog, controller.choice.model)
      : null;
  const selectedEffort = selectedModel?.supportedReasoningEfforts.find(
    (option) => option.reasoningEffort === controller.choice?.reasoningEffort,
  );
  const modelLabel =
    selectedModel?.displayName ??
    controller.choice?.model ??
    (controller.loading ? "Loading models…" : "Codex default");
  const effortLabel = selectedEffort
    ? codeReasoningEffortLabel(selectedEffort.reasoningEffort)
    : controller.choice?.reasoningEffort
      ? codeReasoningEffortLabel(controller.choice.reasoningEffort)
      : selectedModel
        ? `Default · ${codeReasoningEffortLabel(selectedModel.defaultReasoningEffort)}`
        : "Default effort";
  const modelLocked =
    disabled ||
    controller.loading ||
    controller.saving ||
    controller.catalog === null;
  const effortLocked = modelLocked || selectedModel === null;

  React.useEffect(() => {
    if (modelLocked) setModelMenuOpen(false);
    if (effortLocked) setEffortMenuOpen(false);
  }, [effortLocked, modelLocked]);

  return (
    <div className="flex min-w-0 flex-col items-end gap-0.5">
      <div className="flex min-w-0 items-center gap-1">
        <DropdownMenu
          onOpenChange={(open) => {
            if (!open || !modelLocked) setModelMenuOpen(open);
          }}
          open={modelMenuOpen}
        >
          <DropdownMenuTrigger asChild>
            <Button
              aria-busy={controller.loading || controller.saving || undefined}
              aria-disabled={modelLocked}
              aria-label={`Model: ${modelLabel}`}
              className="max-w-44 aria-disabled:cursor-not-allowed aria-disabled:opacity-50"
              data-testid="code-model-selector"
              onKeyDown={(event) => preventLockedMenuOpen(event, modelLocked)}
              onPointerDown={(event) => {
                if (modelLocked) event.preventDefault();
              }}
              size="xs"
              title={
                disabled
                  ? "Model selection is unavailable while this task is busy"
                  : `Model: ${modelLabel}`
              }
              type="button"
              variant="ghost"
            >
              <span className="truncate">{modelLabel}</span>
              {controller.loading || controller.saving ? (
                <LoaderCircle className="animate-spin motion-reduce:animate-none" />
              ) : (
                <ChevronDown />
              )}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="end"
            className={MENU_WIDTH_CLASS}
            sideOffset={6}
            style={{ maxHeight: MENU_MAX_HEIGHT }}
          >
            <DropdownMenuLabel>Model</DropdownMenuLabel>
            <DropdownMenuRadioGroup
              onValueChange={controller.chooseModel}
              value={controller.choice?.model ?? ""}
            >
              {controller.choice && selectedModel === null ? (
                <DropdownMenuRadioItem disabled value={controller.choice.model}>
                  <span className="min-w-0">
                    <span className="block truncate font-medium">
                      {controller.choice.model}
                    </span>
                    <span className="block text-xs text-muted-foreground">
                      Current model is no longer available
                    </span>
                  </span>
                </DropdownMenuRadioItem>
              ) : null}
              {controller.catalog?.models.map((option) => (
                <DropdownMenuRadioItem key={option.id} value={option.model}>
                  <span className="min-w-0">
                    <span className="block truncate font-medium">
                      {option.displayName}
                      {option.isDefault ? (
                        <span className="ml-1 font-normal text-2xs text-muted-foreground">
                          Default
                        </span>
                      ) : null}
                    </span>
                    <span className="block whitespace-normal text-xs text-muted-foreground">
                      {option.description}
                    </span>
                  </span>
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>

        <DropdownMenu
          onOpenChange={(open) => {
            if (!open || !effortLocked) setEffortMenuOpen(open);
          }}
          open={effortMenuOpen}
        >
          <DropdownMenuTrigger asChild>
            <Button
              aria-disabled={effortLocked}
              aria-label={`Reasoning effort: ${effortLabel}`}
              className="max-w-36 aria-disabled:cursor-not-allowed aria-disabled:opacity-50"
              data-testid="code-reasoning-selector"
              onKeyDown={(event) => preventLockedMenuOpen(event, effortLocked)}
              onPointerDown={(event) => {
                if (effortLocked) event.preventDefault();
              }}
              size="xs"
              title={
                disabled
                  ? "Reasoning selection is unavailable while this task is busy"
                  : `Reasoning effort: ${effortLabel}`
              }
              type="button"
              variant="ghost"
            >
              <span className="truncate">{effortLabel}</span>
              <ChevronDown />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="end"
            className="overflow-y-auto"
            sideOffset={6}
            style={{ maxHeight: MENU_MAX_HEIGHT }}
          >
            <DropdownMenuLabel>Reasoning effort</DropdownMenuLabel>
            <DropdownMenuRadioGroup
              onValueChange={controller.chooseReasoningEffort}
              value={selectedEffort?.reasoningEffort ?? ""}
            >
              {selectedModel?.supportedReasoningEfforts.map((option) => (
                <DropdownMenuRadioItem
                  key={option.reasoningEffort}
                  value={option.reasoningEffort}
                >
                  <span className="min-w-0">
                    <span className="block font-medium">
                      {codeReasoningEffortLabel(option.reasoningEffort)}
                    </span>
                    <span className="block whitespace-normal text-xs text-muted-foreground">
                      {option.description}
                    </span>
                  </span>
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      {controller.loading ? (
        <span className="sr-only" role="status">
          Loading Code model options…
        </span>
      ) : null}
      {controller.saving ? (
        <span className="sr-only" role="status">
          Saving Code model selection…
        </span>
      ) : null}
      {controller.error ? (
        <div
          className="flex max-w-md items-center gap-1 text-2xs text-destructive"
          data-testid="code-model-selector-error"
          role="alert"
        >
          <span>{controller.error}</span>
          <Button
            disabled={disabled || controller.loading || controller.saving}
            onClick={controller.retry}
            size="xs"
            type="button"
            variant="ghost"
          >
            Retry
          </Button>
        </div>
      ) : null}
    </div>
  );
}
