import type {
  CodeThreadSummary,
  CodeWorkspaceEvent,
  JsonValue,
} from "../api/types";

type CodeTimelineRowBase = {
  readonly key: string;
  readonly threadId: string;
  readonly turnId: string | null;
  readonly itemId: string | null;
  readonly firstSequence: number | null;
  readonly lastSequence: number | null;
};

/** One render-safe plan step extracted from a normalized Codex value. */
export type CodeTimelinePlanStep = {
  readonly text: string;
  readonly status: string | null;
};

/** One render-safe changed-file summary; patch bodies remain outside the timeline. */
export type CodeTimelineFileChange = {
  readonly path: string;
  readonly changeType: string | null;
};

/**
 * Semantic rows consumed by the minimal SchoolX Code timeline.
 *
 * Deliberately absent: raw app-server payloads, arbitrary JSON values, private
 * reasoning text, and full patch bodies.
 */
export type CodeTimelineRow =
  | (CodeTimelineRowBase & {
      readonly kind: "user";
      readonly text: string;
      readonly pending: boolean;
    })
  | (CodeTimelineRowBase & {
      readonly kind: "agent";
      readonly text: string;
      readonly streaming: boolean;
    })
  | (CodeTimelineRowBase & {
      readonly kind: "plan";
      readonly text: string;
      readonly steps: readonly CodeTimelinePlanStep[];
      readonly streaming: boolean;
    })
  | (CodeTimelineRowBase & {
      readonly kind: "commandOutput";
      readonly command: string | null;
      readonly output: string;
      readonly status: string | null;
      readonly exitCode: number | null;
      readonly streaming: boolean;
    })
  | (CodeTimelineRowBase & {
      readonly kind: "fileChange";
      readonly changes: readonly CodeTimelineFileChange[];
      readonly status: string | null;
      readonly streaming: boolean;
    })
  | (CodeTimelineRowBase & {
      readonly kind: "warning" | "error";
      readonly message: string;
    })
  | (CodeTimelineRowBase & {
      readonly kind: "turnStatus";
      readonly status: string;
    });

/** Optimistic prompt text that has not necessarily appeared in a thread snapshot yet. */
export type CodeTimelineLocalPrompt = {
  readonly id: string;
  readonly text: string;
  readonly turnId?: string | null;
};

type JsonRecord = Readonly<Record<string, JsonValue>>;

type ProjectionContext = {
  readonly key: string;
  readonly threadId: string;
  readonly turnId: string | null;
  readonly itemId: string | null;
  readonly sequence: number | null;
};

type TimelineAccumulator = {
  readonly rows: CodeTimelineRow[];
  readonly rowIndexes: Map<string, number>;
};

type TextUpdate = {
  readonly text: string | null;
  readonly mode: "append" | "replace";
};

function asRecord(value: JsonValue | undefined): JsonRecord | null {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return null;
  }
  return value;
}

function readString(
  record: JsonRecord | null,
  ...keys: readonly string[]
): string | null {
  if (record === null) return null;
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "string") return value;
  }
  return null;
}

function readNumber(
  record: JsonRecord | null,
  ...keys: readonly string[]
): number | null {
  if (record === null) return null;
  for (const key of keys) {
    const value = record[key];
    if (typeof value === "number" && Number.isFinite(value)) return value;
  }
  return null;
}

function textFromContent(value: JsonValue | undefined): string | null {
  if (typeof value === "string") return value;
  if (!Array.isArray(value)) return null;

  const parts: string[] = [];
  for (const part of value) {
    if (typeof part === "string") {
      parts.push(part);
      continue;
    }
    const text = readString(asRecord(part), "text");
    if (text !== null) parts.push(text);
  }
  return parts.length === 0 ? null : parts.join("\n");
}

function readMessageText(record: JsonRecord): string | null {
  return readString(record, "text") ?? textFromContent(record.content);
}

function readReasoningSummary(record: JsonRecord): string | null {
  return textFromContent(record.summary) ?? readString(record, "summaryText");
}

function readPlanSteps(value: JsonValue | undefined): CodeTimelinePlanStep[] {
  if (!Array.isArray(value)) return [];
  const steps: CodeTimelinePlanStep[] = [];
  for (const valueStep of value) {
    const step = asRecord(valueStep);
    const text = readString(step, "step", "text");
    if (text === null) continue;
    steps.push({ text, status: readString(step, "status") });
  }
  return steps;
}

function readFileChanges(
  value: JsonValue | undefined,
): CodeTimelineFileChange[] {
  if (!Array.isArray(value)) return [];
  const changes: CodeTimelineFileChange[] = [];
  for (const valueChange of value) {
    const change = asRecord(valueChange);
    const path = readString(change, "path");
    if (path === null) continue;
    const kind = change === null ? null : asRecord(change.kind);
    changes.push({
      path,
      changeType: readString(change, "changeType") ?? readString(kind, "type"),
    });
  }
  return changes;
}

function readCommand(record: JsonRecord): string | null {
  const value = record.command;
  if (typeof value === "string") return value;
  if (
    Array.isArray(value) &&
    value.length > 0 &&
    value.every((part) => typeof part === "string")
  ) {
    return value.join(" ");
  }
  return null;
}

function readErrorMessage(value: JsonValue | undefined): string | null {
  if (typeof value === "string") return value;
  const record = asRecord(value);
  if (record === null) return null;

  const direct = readString(record, "message", "summary", "details");
  if (direct !== null) return direct;
  return readString(asRecord(record.error), "message");
}

function base(context: ProjectionContext): CodeTimelineRowBase {
  return {
    key: context.key,
    threadId: context.threadId,
    turnId: context.turnId,
    itemId: context.itemId,
    firstSequence: context.sequence,
    lastSequence: context.sequence,
  };
}

function mergedBase(
  current: CodeTimelineRow,
  context: ProjectionContext,
): CodeTimelineRowBase {
  return {
    key: current.key,
    threadId: current.threadId,
    turnId: current.turnId ?? context.turnId,
    itemId: current.itemId ?? context.itemId,
    firstSequence: current.firstSequence ?? context.sequence,
    lastSequence: context.sequence ?? current.lastSequence,
  };
}

function getRow(
  accumulator: TimelineAccumulator,
  key: string,
): CodeTimelineRow | null {
  const index = accumulator.rowIndexes.get(key);
  return index === undefined ? null : (accumulator.rows[index] ?? null);
}

function setRow(
  accumulator: TimelineAccumulator,
  key: string,
  row: CodeTimelineRow,
): void {
  const index = accumulator.rowIndexes.get(key);
  if (index === undefined) {
    accumulator.rowIndexes.set(key, accumulator.rows.length);
    accumulator.rows.push(row);
    return;
  }
  accumulator.rows[index] = row;
}

function addRow(accumulator: TimelineAccumulator, row: CodeTimelineRow): void {
  accumulator.rowIndexes.set(row.key, accumulator.rows.length);
  accumulator.rows.push(row);
}

function updateText(current: string, update: TextUpdate): string {
  if (update.text === null) return current;
  return update.mode === "append" ? `${current}${update.text}` : update.text;
}

function itemKey(
  kind: CodeTimelineRow["kind"],
  turnId: string | null,
  itemId: string | null,
  fallback: string,
): string {
  return itemId === null
    ? `${fallback}:${kind}`
    : JSON.stringify(["item", turnId, itemId, kind]);
}

function upsertUser(
  accumulator: TimelineAccumulator,
  context: ProjectionContext,
  text: string,
  pending: boolean,
): void {
  const current = getRow(accumulator, context.key);
  if (current?.kind === "user") {
    setRow(accumulator, context.key, {
      ...mergedBase(current, context),
      kind: "user",
      text,
      pending: current.pending && pending,
    });
    return;
  }
  setRow(accumulator, context.key, {
    ...base(context),
    kind: "user",
    text,
    pending,
  });
}

function upsertAgent(
  accumulator: TimelineAccumulator,
  context: ProjectionContext,
  update: TextUpdate,
  streaming: boolean,
  allowEmpty = false,
): void {
  const current = getRow(accumulator, context.key);
  if (current?.kind === "agent") {
    setRow(accumulator, context.key, {
      ...mergedBase(current, context),
      kind: "agent",
      text: updateText(current.text, update),
      streaming,
    });
    return;
  }
  if (!allowEmpty && (update.text === null || update.text.length === 0)) return;
  setRow(accumulator, context.key, {
    ...base(context),
    kind: "agent",
    text: update.text ?? "",
    streaming,
  });
}

function upsertPlan(
  accumulator: TimelineAccumulator,
  context: ProjectionContext,
  update: TextUpdate,
  steps: readonly CodeTimelinePlanStep[] | undefined,
  streaming: boolean,
  allowEmpty = false,
): void {
  const current = getRow(accumulator, context.key);
  if (current?.kind === "plan") {
    setRow(accumulator, context.key, {
      ...mergedBase(current, context),
      kind: "plan",
      text: updateText(current.text, update),
      steps: steps ?? current.steps,
      streaming,
    });
    return;
  }
  if (
    !allowEmpty &&
    (update.text === null || update.text.length === 0) &&
    (steps === undefined || steps.length === 0)
  ) {
    return;
  }
  setRow(accumulator, context.key, {
    ...base(context),
    kind: "plan",
    text: update.text ?? "",
    steps: steps ?? [],
    streaming,
  });
}

type CommandUpdate = {
  readonly command?: string | null;
  readonly output?: TextUpdate;
  readonly status?: string | null;
  readonly exitCode?: number | null;
  readonly streaming: boolean;
};

function upsertCommand(
  accumulator: TimelineAccumulator,
  context: ProjectionContext,
  update: CommandUpdate,
): void {
  const current = getRow(accumulator, context.key);
  if (current?.kind === "commandOutput") {
    setRow(accumulator, context.key, {
      ...mergedBase(current, context),
      kind: "commandOutput",
      command: update.command === undefined ? current.command : update.command,
      output:
        update.output === undefined
          ? current.output
          : updateText(current.output, update.output),
      status: update.status === undefined ? current.status : update.status,
      exitCode:
        update.exitCode === undefined ? current.exitCode : update.exitCode,
      streaming: update.streaming,
    });
    return;
  }
  setRow(accumulator, context.key, {
    ...base(context),
    kind: "commandOutput",
    command: update.command ?? null,
    output: update.output?.text ?? "",
    status: update.status ?? null,
    exitCode: update.exitCode ?? null,
    streaming: update.streaming,
  });
}

type FileChangeUpdate = {
  readonly changes?: readonly CodeTimelineFileChange[];
  readonly status?: string | null;
  readonly streaming: boolean;
};

function mergeFileChanges(
  current: readonly CodeTimelineFileChange[],
  incoming: readonly CodeTimelineFileChange[],
): readonly CodeTimelineFileChange[] {
  const byPath = new Map(current.map((change) => [change.path, change]));
  for (const change of incoming) byPath.set(change.path, change);
  return [...byPath.values()];
}

function upsertFileChange(
  accumulator: TimelineAccumulator,
  context: ProjectionContext,
  update: FileChangeUpdate,
): void {
  const current = getRow(accumulator, context.key);
  if (current?.kind === "fileChange") {
    setRow(accumulator, context.key, {
      ...mergedBase(current, context),
      kind: "fileChange",
      changes:
        update.changes === undefined
          ? current.changes
          : mergeFileChanges(current.changes, update.changes),
      status: update.status === undefined ? current.status : update.status,
      streaming: update.streaming,
    });
    return;
  }
  setRow(accumulator, context.key, {
    ...base(context),
    kind: "fileChange",
    changes: update.changes ?? [],
    status: update.status ?? null,
    streaming: update.streaming,
  });
}

function appendNotice(
  accumulator: TimelineAccumulator,
  context: ProjectionContext,
  kind: "warning" | "error",
  message: string,
): void {
  addRow(accumulator, { ...base(context), kind, message });
}

function appendTurnStatus(
  accumulator: TimelineAccumulator,
  context: ProjectionContext,
  status: string,
): void {
  addRow(accumulator, { ...base(context), kind: "turnStatus", status });
}

function projectItem(
  accumulator: TimelineAccumulator,
  item: JsonRecord,
  context: Omit<ProjectionContext, "key" | "itemId"> & {
    readonly fallback: string;
    readonly envelopeItemId?: string | null;
  },
  streaming: boolean,
): void {
  const type = readString(item, "type");
  const itemId = context.envelopeItemId ?? readString(item, "id");
  if (type === null) return;

  const projectionContext = (
    kind: CodeTimelineRow["kind"],
  ): ProjectionContext => ({
    threadId: context.threadId,
    turnId: context.turnId,
    itemId,
    sequence: context.sequence,
    key: itemKey(kind, context.turnId, itemId, context.fallback),
  });

  switch (type) {
    case "userMessage": {
      const text = readMessageText(item);
      if (text !== null) {
        upsertUser(accumulator, projectionContext("user"), text, false);
      }
      return;
    }
    case "agentMessage":
      upsertAgent(
        accumulator,
        projectionContext("agent"),
        { text: readMessageText(item), mode: "replace" },
        streaming,
        true,
      );
      return;
    case "plan":
      upsertPlan(
        accumulator,
        projectionContext("plan"),
        { text: readString(item, "text", "explanation"), mode: "replace" },
        readPlanSteps(item.steps ?? item.plan),
        streaming,
        true,
      );
      return;
    case "reasoning":
      upsertPlan(
        accumulator,
        projectionContext("plan"),
        { text: readReasoningSummary(item), mode: "replace" },
        undefined,
        streaming,
      );
      return;
    case "commandExecution":
      upsertCommand(accumulator, projectionContext("commandOutput"), {
        command: readCommand(item),
        output: {
          text: readString(item, "aggregatedOutput", "output"),
          mode: "replace",
        },
        status: readString(item, "status"),
        exitCode: readNumber(item, "exitCode"),
        streaming,
      });
      return;
    case "fileChange":
      upsertFileChange(accumulator, projectionContext("fileChange"), {
        changes: readFileChanges(item.changes),
        status: readString(item, "status"),
        streaming,
      });
  }
}

function projectSnapshot(
  accumulator: TimelineAccumulator,
  thread: CodeThreadSummary,
): void {
  thread.turns.forEach((turn, turnIndex) => {
    turn.items.forEach((value, itemIndex) => {
      const item = asRecord(value);
      if (item === null) return;
      projectItem(
        accumulator,
        item,
        {
          threadId: thread.id,
          turnId: turn.id,
          sequence: null,
          fallback: `snapshot:${turnIndex}:${itemIndex}`,
        },
        false,
      );
    });

    const errorMessage = readErrorMessage(turn.error ?? undefined);
    if (errorMessage !== null) {
      appendNotice(
        accumulator,
        {
          key: `snapshot:${turnIndex}:error`,
          threadId: thread.id,
          turnId: turn.id,
          itemId: null,
          sequence: null,
        },
        "error",
        errorMessage,
      );
    }
    if (turn.status.length > 0) {
      appendTurnStatus(
        accumulator,
        {
          key: `snapshot:${turnIndex}:status`,
          threadId: thread.id,
          turnId: turn.id,
          itemId: null,
          sequence: null,
        },
        turn.status,
      );
    }
  });
}

function projectLocalPrompts(
  accumulator: TimelineAccumulator,
  threadId: string,
  prompts: readonly CodeTimelineLocalPrompt[],
): void {
  prompts.forEach((prompt, index) => {
    if (prompt.text.length === 0) return;
    const turnId = prompt.turnId ?? null;
    const key = itemKey("user", turnId, prompt.id, `local:${index}`);
    upsertUser(
      accumulator,
      {
        key,
        threadId,
        turnId,
        itemId: prompt.id,
        sequence: null,
      },
      prompt.text,
      turnId === null,
    );
  });
}

function eventFallback(event: CodeWorkspaceEvent): string {
  return `event:${event.runtimeGeneration}:${event.sequence}`;
}

function eventContext(
  event: CodeWorkspaceEvent,
  kind: CodeTimelineRow["kind"],
): ProjectionContext {
  const fallback = eventFallback(event);
  return {
    key: itemKey(kind, event.turnId, event.itemId, fallback),
    threadId: event.threadId ?? "",
    turnId: event.turnId,
    itemId: event.itemId,
    sequence: event.sequence,
  };
}

function projectEvent(
  accumulator: TimelineAccumulator,
  event: CodeWorkspaceEvent,
): void {
  if (event.kind === "item/reasoning/textDelta") return;
  const payload = asRecord(event.payload);

  switch (event.kind) {
    case "item/started":
    case "item/completed": {
      const item = payload === null ? null : asRecord(payload.item);
      if (item === null) return;
      projectItem(
        accumulator,
        item,
        {
          threadId: event.threadId ?? "",
          turnId: event.turnId,
          sequence: event.sequence,
          fallback: eventFallback(event),
          envelopeItemId: event.itemId,
        },
        event.kind === "item/started",
      );
      return;
    }
    case "item/agentMessage/delta":
      upsertAgent(
        accumulator,
        eventContext(event, "agent"),
        { text: readString(payload, "delta"), mode: "append" },
        true,
      );
      return;
    case "item/plan/delta":
    case "item/reasoning/summaryTextDelta":
      upsertPlan(
        accumulator,
        eventContext(event, "plan"),
        { text: readString(payload, "delta"), mode: "append" },
        undefined,
        true,
      );
      return;
    case "turn/plan/updated": {
      const context = eventContext(event, "plan");
      upsertPlan(
        accumulator,
        {
          ...context,
          key: JSON.stringify(["turn-plan", event.turnId]),
        },
        { text: readString(payload, "explanation"), mode: "replace" },
        readPlanSteps(payload?.plan),
        false,
      );
      return;
    }
    case "item/commandExecution/outputDelta":
      upsertCommand(accumulator, eventContext(event, "commandOutput"), {
        output: { text: readString(payload, "delta"), mode: "append" },
        streaming: true,
      });
      return;
    case "item/fileChange/patchUpdated":
      upsertFileChange(accumulator, eventContext(event, "fileChange"), {
        changes: readFileChanges(payload?.changes),
        streaming: true,
      });
      return;
    case "turn/diff/updated": {
      const context = eventContext(event, "fileChange");
      upsertFileChange(
        accumulator,
        {
          ...context,
          key: JSON.stringify(["turn-diff", event.turnId]),
        },
        { streaming: true },
      );
      return;
    }
    case "warning":
    case "configWarning":
      appendNotice(
        accumulator,
        {
          ...eventContext(event, "warning"),
          key: `${eventFallback(event)}:warning`,
        },
        "warning",
        readErrorMessage(event.payload) ?? "Codex reported a warning.",
      );
      return;
    case "error":
      appendNotice(
        accumulator,
        {
          ...eventContext(event, "error"),
          key: `${eventFallback(event)}:error`,
        },
        "error",
        readErrorMessage(event.payload) ?? "Codex reported an error.",
      );
      return;
    case "turn/started":
    case "turn/completed": {
      const turn = payload === null ? null : asRecord(payload.turn);
      const status =
        readString(turn, "status") ??
        (event.kind === "turn/started" ? "inProgress" : "completed");
      appendTurnStatus(
        accumulator,
        {
          ...eventContext(event, "turnStatus"),
          key: `${eventFallback(event)}:${event.kind}`,
        },
        status,
      );
      if (event.kind === "turn/completed") {
        const message = readErrorMessage(turn?.error);
        if (message !== null) {
          appendNotice(
            accumulator,
            {
              ...eventContext(event, "error"),
              key: `${eventFallback(event)}:turn-error`,
            },
            "error",
            message,
          );
        }
      }
      return;
    }
  }
}

/**
 * Project a restored thread and its normalized live events into semantic rows.
 *
 * The projection is deterministic and side-effect free. Events for any other
 * thread are ignored, duplicate generation/sequence envelopes are folded once,
 * and compatible item deltas share one stable row.
 */
export function projectCodeTimeline(
  thread: CodeThreadSummary,
  events: readonly CodeWorkspaceEvent[],
  localPrompts: readonly CodeTimelineLocalPrompt[] = [],
): readonly CodeTimelineRow[] {
  const accumulator: TimelineAccumulator = {
    rows: [],
    rowIndexes: new Map(),
  };
  projectSnapshot(accumulator, thread);
  projectLocalPrompts(accumulator, thread.id, localPrompts);

  const seenEvents = new Set<string>();
  const selectedEvents = events
    .map((event, inputIndex) => ({ event, inputIndex }))
    .filter(({ event }) => event.threadId === thread.id)
    .sort(
      (left, right) =>
        left.event.sequence - right.event.sequence ||
        left.inputIndex - right.inputIndex,
    );
  for (const { event } of selectedEvents) {
    const identity = `${event.runtimeGeneration}:${event.sequence}`;
    if (seenEvents.has(identity)) continue;
    seenEvents.add(identity);
    projectEvent(accumulator, event);
  }

  return accumulator.rows;
}
