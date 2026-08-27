export const CODE_TASKS_PANEL_DEFAULT_WIDTH_PX = 240;
export const CODE_TASKS_PANEL_MIN_WIDTH_PX = 200;
export const CODE_TASKS_PANEL_MAX_WIDTH_PX = 420;
export const CODE_CHANGES_PANEL_DEFAULT_WIDTH_PX = 544;
export const CODE_CHANGES_PANEL_MIN_WIDTH_PX = 320;
export const CODE_CHANGES_PANEL_MAX_WIDTH_PX = 720;
export const CODE_CONVERSATION_MIN_WIDTH_PX = 360;
export const CODE_CHANGES_OVERLAY_GUTTER_PX = 48;
export const CODE_PANEL_KEYBOARD_STEP_PX = 16;

const CODE_PANEL_WIDTHS_STORAGE_KEY = "buzz.desktop.schoolx-code-panel-widths";

export type CodePanelWidths = {
  tasks: number;
  changes: number;
};

export type CodePanelLayout = {
  changes: { width: number; min: number; max: number };
  inspectorDocked: boolean;
  tasks: { width: number; min: number; max: number };
};

function clamp(width: number, min: number, max: number): number {
  return Math.round(Math.min(max, Math.max(min, width)));
}

function finiteWidth(value: unknown, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

/** Read device-local Code pane preferences without trusting stored JSON. */
export function readCodePanelWidths(
  storage: Pick<Storage, "getItem"> | null | undefined,
): CodePanelWidths {
  if (!storage) {
    return {
      tasks: CODE_TASKS_PANEL_DEFAULT_WIDTH_PX,
      changes: CODE_CHANGES_PANEL_DEFAULT_WIDTH_PX,
    };
  }
  try {
    const raw = storage.getItem(CODE_PANEL_WIDTHS_STORAGE_KEY);
    if (!raw) throw new Error("missing Code panel widths");
    const parsed = JSON.parse(raw) as Record<string, unknown>;
    return {
      tasks: clamp(
        finiteWidth(parsed.tasks, CODE_TASKS_PANEL_DEFAULT_WIDTH_PX),
        CODE_TASKS_PANEL_MIN_WIDTH_PX,
        CODE_TASKS_PANEL_MAX_WIDTH_PX,
      ),
      changes: clamp(
        finiteWidth(parsed.changes, CODE_CHANGES_PANEL_DEFAULT_WIDTH_PX),
        CODE_CHANGES_PANEL_MIN_WIDTH_PX,
        CODE_CHANGES_PANEL_MAX_WIDTH_PX,
      ),
    };
  } catch {
    return {
      tasks: CODE_TASKS_PANEL_DEFAULT_WIDTH_PX,
      changes: CODE_CHANGES_PANEL_DEFAULT_WIDTH_PX,
    };
  }
}

/** Persist both pane preferences as one atomic device-local snapshot. */
export function writeCodePanelWidths(
  storage: Pick<Storage, "setItem"> | null | undefined,
  widths: CodePanelWidths,
): void {
  storage?.setItem(CODE_PANEL_WIDTHS_STORAGE_KEY, JSON.stringify(widths));
}

/**
 * Resolve responsive pane widths while preserving useful task, conversation,
 * and Changes regions. Changes becomes an overlay when all three minima do
 * not fit in the measured Code workspace.
 */
export function resolveCodePanelLayout({
  changesOpen,
  containerWidth,
  preferred,
  tasksOpen,
}: {
  changesOpen: boolean;
  containerWidth: number;
  preferred: CodePanelWidths;
  tasksOpen: boolean;
}): CodePanelLayout {
  const available = Math.max(0, Math.floor(containerWidth));
  const dockMinimum =
    CODE_CONVERSATION_MIN_WIDTH_PX +
    CODE_CHANGES_PANEL_MIN_WIDTH_PX +
    (tasksOpen ? CODE_TASKS_PANEL_MIN_WIDTH_PX : 0);
  const inspectorDocked = changesOpen && available >= dockMinimum;
  const taskCapacity = Math.max(
    0,
    available -
      CODE_CONVERSATION_MIN_WIDTH_PX -
      (inspectorDocked ? CODE_CHANGES_PANEL_MIN_WIDTH_PX : 0),
  );
  const tasksMin = tasksOpen
    ? Math.min(CODE_TASKS_PANEL_MIN_WIDTH_PX, available)
    : 0;
  const tasksMax = tasksOpen
    ? Math.max(tasksMin, Math.min(CODE_TASKS_PANEL_MAX_WIDTH_PX, taskCapacity))
    : 0;
  const tasksWidth = tasksOpen ? clamp(preferred.tasks, tasksMin, tasksMax) : 0;
  const changesCapacity = inspectorDocked
    ? available - CODE_CONVERSATION_MIN_WIDTH_PX - (tasksOpen ? tasksWidth : 0)
    : available - (tasksOpen ? tasksWidth : 0) - CODE_CHANGES_OVERLAY_GUTTER_PX;
  const changesMax = Math.min(
    CODE_CHANGES_PANEL_MAX_WIDTH_PX,
    Math.max(0, changesCapacity),
  );
  const changesMin = Math.min(CODE_CHANGES_PANEL_MIN_WIDTH_PX, changesMax);
  const changesWidth = changesOpen
    ? clamp(preferred.changes, changesMin, changesMax)
    : 0;

  return {
    changes: { width: changesWidth, min: changesMin, max: changesMax },
    inspectorDocked,
    tasks: { width: tasksWidth, min: tasksMin, max: tasksMax },
  };
}

/** Translate a captured pointer delta directly into a clamped pane width. */
export function codePanelWidthFromPointer({
  currentX,
  growDirection,
  max,
  min,
  startWidth,
  startX,
}: {
  currentX: number;
  growDirection: 1 | -1;
  max: number;
  min: number;
  startWidth: number;
  startX: number;
}): number {
  return clamp(startWidth + (currentX - startX) * growDirection, min, max);
}

/** Map standard separator keys to the same physical left/right movement. */
export function codePanelWidthFromKey({
  growDirection,
  key,
  max,
  min,
  width,
}: {
  growDirection: 1 | -1;
  key: string;
  max: number;
  min: number;
  width: number;
}): number | null {
  switch (key) {
    case "ArrowLeft":
      return clamp(
        width - CODE_PANEL_KEYBOARD_STEP_PX * growDirection,
        min,
        max,
      );
    case "ArrowRight":
      return clamp(
        width + CODE_PANEL_KEYBOARD_STEP_PX * growDirection,
        min,
        max,
      );
    case "Home":
      return growDirection === 1 ? min : max;
    case "End":
      return growDirection === 1 ? max : min;
    default:
      return null;
  }
}
