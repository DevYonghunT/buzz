import type {
  CodeModelOption,
  CodeModelSelection,
  CodeModelsCatalog,
} from "../api/types";

/** Display state may retain an unavailable or effort-unknown opened thread. */
export type CodeModelChoice = {
  model: string;
  reasoningEffort: string | null;
};

/** The visible choice and the exact pair safe to send on the next turn. */
export type CodeResolvedModelSelection = {
  choice: CodeModelChoice;
  turnSelection: CodeModelSelection | null;
};

export function findCodeModelOption(
  catalog: CodeModelsCatalog,
  model: string,
): CodeModelOption | null {
  return catalog.models.find((option) => option.model === model) ?? null;
}

export function isCodeReasoningEffortSupported(
  option: CodeModelOption,
  reasoningEffort: string,
): boolean {
  return option.supportedReasoningEfforts.some(
    (effort) => effort.reasoningEffort === reasoningEffort,
  );
}

/** Resolve recent selection, then the advertised default, then first model. */
export function defaultCodeModelSelection(
  catalog: CodeModelsCatalog,
): CodeModelSelection | null {
  if (catalog.recentSelection !== null) {
    const recentModel = findCodeModelOption(
      catalog,
      catalog.recentSelection.model,
    );
    if (
      recentModel !== null &&
      isCodeReasoningEffortSupported(
        recentModel,
        catalog.recentSelection.reasoningEffort,
      )
    ) {
      return catalog.recentSelection;
    }
  }
  const model =
    catalog.models.find((option) => option.isDefault) ??
    catalog.models.at(0) ??
    null;
  return model === null
    ? null
    : {
        model: model.model,
        reasoningEffort: model.defaultReasoningEffort,
      };
}

/**
 * Preserve exact open authority. Unknown effort or an unavailable model stays
 * visible, but is not converted into a turn override without user intent.
 */
export function codeModelSelectionFromOpen(
  catalog: CodeModelsCatalog | null,
  choice: CodeModelChoice,
): CodeResolvedModelSelection {
  const option = catalog ? findCodeModelOption(catalog, choice.model) : null;
  const knownPair =
    option !== null &&
    choice.reasoningEffort !== null &&
    isCodeReasoningEffortSupported(option, choice.reasoningEffort);
  return {
    choice,
    turnSelection: knownPair
      ? {
          model: choice.model,
          reasoningEffort: choice.reasoningEffort as string,
        }
      : null,
  };
}

/** Preserve an explicit pre-start effort only when native opened that model. */
export function codeModelSelectionFromFreshOpen(
  catalog: CodeModelsCatalog | null,
  choice: CodeModelChoice,
  pendingSelection: CodeModelSelection | null,
): CodeResolvedModelSelection {
  if (
    catalog !== null &&
    pendingSelection !== null &&
    pendingSelection.model === choice.model
  ) {
    const option = findCodeModelOption(catalog, choice.model);
    if (
      option !== null &&
      isCodeReasoningEffortSupported(option, pendingSelection.reasoningEffort)
    ) {
      return {
        choice: pendingSelection,
        turnSelection: pendingSelection,
      };
    }
  }
  return codeModelSelectionFromOpen(catalog, choice);
}

/** Model changes retain effort when supported, otherwise choose its default. */
export function selectCodeModel(
  catalog: CodeModelsCatalog,
  current: CodeModelChoice | null,
  model: string,
): CodeModelSelection | null {
  const option = findCodeModelOption(catalog, model);
  if (option === null) return null;
  const reasoningEffort =
    current?.reasoningEffort !== null &&
    current?.reasoningEffort !== undefined &&
    isCodeReasoningEffortSupported(option, current.reasoningEffort)
      ? current.reasoningEffort
      : option.defaultReasoningEffort;
  return { model: option.model, reasoningEffort };
}

/** Accept effort only when the current catalog model advertises it. */
export function selectCodeReasoningEffort(
  catalog: CodeModelsCatalog,
  current: CodeModelChoice | null,
  reasoningEffort: string,
): CodeModelSelection | null {
  if (current === null) return null;
  const option = findCodeModelOption(catalog, current.model);
  return option !== null &&
    isCodeReasoningEffortSupported(option, reasoningEffort)
    ? { model: option.model, reasoningEffort }
    : null;
}

const REASONING_EFFORT_LABELS: Readonly<Record<string, string>> = {
  minimal: "Minimal",
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "Extra high",
  max: "Max",
  ultra: "Ultra",
};

/** Stable human-readable copy with a safe future-value fallback. */
export function codeReasoningEffortLabel(reasoningEffort: string): string {
  const known = REASONING_EFFORT_LABELS[reasoningEffort];
  if (known) return known;
  const formatted = reasoningEffort
    .split(/[-_]/u)
    .filter(Boolean)
    .map((part) => `${part[0]?.toUpperCase() ?? ""}${part.slice(1)}`)
    .join(" ");
  return formatted || reasoningEffort;
}
