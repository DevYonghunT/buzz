import type { CodeWorkspaceReplayBatch } from "../api/codeWorkspace";
import type {
  CodeApprovalResponseInput,
  CodeRequestId,
  CodeRuntimeEventsInput,
  CodeRuntimeStatus,
  CodeThreadBindingScope,
  CodeTurnInterruptInput,
  CodeWorkspaceEvent,
  JsonObject,
  JsonValue,
} from "../api/types";
import { codeScopesEqual } from "../api/types";

const MAX_RETAINED_EVENTS = 512;

export type CodeReplayStatus =
  | "idle"
  | "synchronizing"
  | "synchronized"
  | "truncated"
  | "invalid";

/** Replay completeness for the current desktop-wide runtime generation. */
export type CodeReplayState = {
  readonly status: CodeReplayStatus;
  readonly subscriptionEpoch: number | null;
  readonly request: CodeRuntimeEventsInput | null;
  readonly needsAuthoritativeRefresh: boolean;
  readonly approvalStateIncomplete: boolean;
};

/** Active turn derived only from normalized native lifecycle events. */
export type CodeActiveTurn = {
  runtimeGeneration: number;
  threadId: string;
  turnId: string;
  status: string;
  startedSequence: number;
};

export type CodeApprovalKind =
  | "commandExecution"
  | "fileChange"
  | "permissions";

/** Approval identity and redacted request retained for a future inline card. */
export type CodePendingApproval = {
  runtimeGeneration: number;
  requestId: CodeRequestId;
  scope: CodeThreadBindingScope;
  threadId: string;
  turnId: string;
  itemId: string;
  approvalKind: CodeApprovalKind;
  request: JsonObject;
  sequence: number;
  /** False while native has reserved the response for an in-flight write. */
  respondable: boolean;
};

/** Pure, scope-owned live state for one SchoolX Code workspace. */
export type CodeSessionState = {
  scope: CodeThreadBindingScope;
  runtimeStatus: CodeRuntimeStatus | null;
  runtimeStatusRevision: number;
  runtimeGeneration: number | null;
  latestSequence: number;
  replay: CodeReplayState;
  events: readonly CodeWorkspaceEvent[];
  activeTurns: ReadonlyMap<string, CodeActiveTurn>;
  pendingApprovals: ReadonlyMap<string, CodePendingApproval>;
};

/** Actions accepted by the pure Code session reducer. */
export type CodeSessionAction =
  | {
      type: "runtimeStatusReceived";
      revision: number;
      status: CodeRuntimeStatus;
    }
  | {
      type: "subscriptionStarted";
      subscriptionEpoch: number;
      input: CodeRuntimeEventsInput;
    }
  | {
      type: "eventReceived";
      subscriptionEpoch: number;
      event: CodeWorkspaceEvent;
    }
  | { type: "replayReceived"; batch: CodeWorkspaceReplayBatch }
  | {
      type: "approvalResponseCommitted";
      input: CodeApprovalResponseInput;
      expectedSequence: number;
      expectedItemId: string;
    }
  | {
      type: "turnInterruptCommitted";
      runtimeGeneration: number;
      input: CodeTurnInterruptInput;
    }
  | {
      type: "authoritativeRefreshCompleted";
      runtimeGeneration: number;
      subscriptionEpoch: number;
    }
  | { type: "reset" };

function createEmptyReplay(
  subscriptionEpoch: number | null = null,
): CodeReplayState {
  return {
    status: "idle",
    subscriptionEpoch,
    request: null,
    needsAuthoritativeRefresh: false,
    approvalStateIncomplete: false,
  };
}

/** Create isolated reducer state without installing a module-level singleton. */
export function createCodeSessionState(
  scope: CodeThreadBindingScope,
): CodeSessionState {
  return {
    scope: { ...scope },
    runtimeStatus: null,
    runtimeStatusRevision: 0,
    runtimeGeneration: null,
    latestSequence: 0,
    replay: createEmptyReplay(),
    events: [],
    activeTurns: new Map(),
    pendingApprovals: new Map(),
  };
}

function resetForGeneration(
  state: CodeSessionState,
  runtimeGeneration: number,
  runtimeStatus: CodeRuntimeStatus | null = null,
  runtimeStatusRevision = state.runtimeStatusRevision,
): CodeSessionState {
  return {
    ...state,
    runtimeStatus,
    runtimeStatusRevision,
    runtimeGeneration,
    latestSequence: 0,
    replay: createEmptyReplay(state.replay.subscriptionEpoch),
    events: [],
    activeTurns: new Map(),
    pendingApprovals: new Map(),
  };
}

function activeTurnKey(threadId: string, turnId: string): string {
  return JSON.stringify([threadId, turnId]);
}

/** Stable key preserving numeric versus string JSON-RPC request ids. */
export function codeApprovalIdentityKey(
  identity: Pick<
    CodePendingApproval,
    "runtimeGeneration" | "requestId" | "scope" | "threadId" | "turnId"
  >,
): string {
  return JSON.stringify([
    identity.runtimeGeneration,
    typeof identity.requestId,
    identity.requestId,
    identity.scope.communityId,
    identity.scope.projectDtag,
    identity.scope.repositoryIdentity,
    identity.threadId,
    identity.turnId,
  ]);
}

function isJsonObject(value: JsonValue | undefined): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isRequestId(value: JsonValue | undefined): value is CodeRequestId {
  return (
    typeof value === "string" ||
    (typeof value === "number" && Number.isSafeInteger(value) && value >= 0)
  );
}

function requestIdsEqual(left: CodeRequestId, right: CodeRequestId): boolean {
  return typeof left === typeof right && left === right;
}

function approvalKindForEvent(
  event: CodeWorkspaceEvent,
): CodeApprovalKind | null {
  switch (event.kind) {
    case "item/commandExecution/requestApproval":
      return "commandExecution";
    case "item/fileChange/requestApproval":
      return "fileChange";
    case "item/permissions/requestApproval":
      return "permissions";
    default:
      return null;
  }
}

function pendingApprovalFromEvent(
  event: CodeWorkspaceEvent,
  respondable = true,
): CodePendingApproval | null {
  const expectedKind = approvalKindForEvent(event);
  if (
    expectedKind === null ||
    event.threadId === null ||
    event.turnId === null ||
    event.itemId === null ||
    !isJsonObject(event.payload)
  ) {
    return null;
  }
  const { requestId, approvalKind, request } = event.payload;
  if (
    !isRequestId(requestId) ||
    approvalKind !== expectedKind ||
    !isJsonObject(request) ||
    request.threadId !== event.threadId ||
    request.turnId !== event.turnId ||
    request.itemId !== event.itemId
  ) {
    return null;
  }
  return {
    runtimeGeneration: event.runtimeGeneration,
    requestId,
    scope: event.scope,
    threadId: event.threadId,
    turnId: event.turnId,
    itemId: event.itemId,
    approvalKind: expectedKind,
    request,
    sequence: event.sequence,
    respondable,
  };
}

function nestedTurnStatus(payload: JsonValue): string {
  if (!isJsonObject(payload) || !isJsonObject(payload.turn)) {
    return "inProgress";
  }
  return typeof payload.turn.status === "string"
    ? payload.turn.status
    : "inProgress";
}

function removeTurnState(
  activeTurns: ReadonlyMap<string, CodeActiveTurn>,
  pendingApprovals: ReadonlyMap<string, CodePendingApproval>,
  threadId: string,
  turnId: string,
): {
  activeTurns: ReadonlyMap<string, CodeActiveTurn>;
  pendingApprovals: ReadonlyMap<string, CodePendingApproval>;
} {
  const activeKey = activeTurnKey(threadId, turnId);
  let nextActiveTurns = activeTurns;
  if (activeTurns.has(activeKey)) {
    const mutableActiveTurns = new Map(activeTurns);
    mutableActiveTurns.delete(activeKey);
    nextActiveTurns = mutableActiveTurns;
  }

  let nextPendingApprovals = pendingApprovals;
  for (const [key, approval] of pendingApprovals) {
    if (approval.threadId === threadId && approval.turnId === turnId) {
      if (nextPendingApprovals === pendingApprovals) {
        nextPendingApprovals = new Map(pendingApprovals);
      }
      (nextPendingApprovals as Map<string, CodePendingApproval>).delete(key);
    }
  }
  return {
    activeTurns: nextActiveTurns,
    pendingApprovals: nextPendingApprovals,
  };
}

function deriveEventState(
  state: CodeSessionState,
  event: CodeWorkspaceEvent,
): Pick<CodeSessionState, "activeTurns" | "pendingApprovals"> {
  let activeTurns = state.activeTurns;
  let pendingApprovals = state.pendingApprovals;

  if (
    event.kind === "turn/started" &&
    event.threadId !== null &&
    event.turnId !== null
  ) {
    activeTurns = new Map(activeTurns).set(
      activeTurnKey(event.threadId, event.turnId),
      {
        runtimeGeneration: event.runtimeGeneration,
        threadId: event.threadId,
        turnId: event.turnId,
        status: nestedTurnStatus(event.payload),
        startedSequence: event.sequence,
      },
    );
  }

  if (
    event.kind === "turn/completed" &&
    event.threadId !== null &&
    event.turnId !== null
  ) {
    ({ activeTurns, pendingApprovals } = removeTurnState(
      activeTurns,
      pendingApprovals,
      event.threadId,
      event.turnId,
    ));
  }

  if (event.kind === "thread/closed" && event.threadId !== null) {
    for (const [key, turn] of activeTurns) {
      if (turn.threadId === event.threadId) {
        if (activeTurns === state.activeTurns) {
          activeTurns = new Map(activeTurns);
        }
        (activeTurns as Map<string, CodeActiveTurn>).delete(key);
      }
    }
    for (const [key, approval] of pendingApprovals) {
      if (approval.threadId === event.threadId) {
        if (pendingApprovals === state.pendingApprovals) {
          pendingApprovals = new Map(pendingApprovals);
        }
        (pendingApprovals as Map<string, CodePendingApproval>).delete(key);
      }
    }
  }

  const approval = pendingApprovalFromEvent(event);
  if (approval !== null) {
    pendingApprovals = new Map(pendingApprovals).set(
      codeApprovalIdentityKey(approval),
      approval,
    );
  }

  if (
    event.kind === "serverRequest/resolved" &&
    event.threadId !== null &&
    isJsonObject(event.payload) &&
    isRequestId(event.payload.requestId)
  ) {
    const requestId = event.payload.requestId;
    for (const [key, pending] of pendingApprovals) {
      if (
        pending.runtimeGeneration === event.runtimeGeneration &&
        pending.threadId === event.threadId &&
        requestIdsEqual(pending.requestId, requestId)
      ) {
        if (pendingApprovals === state.pendingApprovals) {
          pendingApprovals = new Map(pendingApprovals);
        }
        (pendingApprovals as Map<string, CodePendingApproval>).delete(key);
      }
    }
  }

  return { activeTurns, pendingApprovals };
}

function canDeriveTransientState(
  state: CodeSessionState,
  runtimeGeneration: number,
): boolean {
  return !(
    state.runtimeStatus?.generation === runtimeGeneration &&
    state.runtimeStatus.phase !== "ready"
  );
}

function applyEvent(
  state: CodeSessionState,
  event: CodeWorkspaceEvent,
  subscriptionEpoch: number,
): CodeSessionState {
  if (
    state.replay.subscriptionEpoch !== subscriptionEpoch ||
    !codeScopesEqual(state.scope, event.scope)
  ) {
    return state;
  }

  let next = state;
  if (
    next.runtimeGeneration === null ||
    event.runtimeGeneration > next.runtimeGeneration
  ) {
    next = resetForGeneration(next, event.runtimeGeneration);
  } else if (event.runtimeGeneration < next.runtimeGeneration) {
    return state;
  }

  if (event.sequence <= next.latestSequence) return next;

  const events = [...next.events, event].slice(-MAX_RETAINED_EVENTS);
  const derived = canDeriveTransientState(next, event.runtimeGeneration)
    ? deriveEventState(next, event)
    : {
        activeTurns: next.activeTurns,
        pendingApprovals: next.pendingApprovals,
      };
  return {
    ...next,
    latestSequence: event.sequence,
    events,
    activeTurns: derived.activeTurns,
    pendingApprovals: derived.pendingApprovals,
  };
}

function compareEvents(left: CodeWorkspaceEvent, right: CodeWorkspaceEvent) {
  return left.runtimeGeneration !== right.runtimeGeneration
    ? left.runtimeGeneration - right.runtimeGeneration
    : left.sequence - right.sequence;
}

function isSafeUnsignedInteger(value: number): boolean {
  return Number.isSafeInteger(value) && value >= 0;
}

function replayBatchIsValid(
  state: CodeSessionState,
  batch: CodeWorkspaceReplayBatch,
): boolean {
  if (
    !isSafeUnsignedInteger(batch.subscriptionEpoch) ||
    !codeScopesEqual(state.scope, batch.request.scope) ||
    !isSafeUnsignedInteger(batch.backlog.runtimeGeneration) ||
    !isSafeUnsignedInteger(batch.backlog.latestSequence)
  ) {
    return false;
  }

  const { runtimeGeneration, afterSequence } = batch.request;
  if (
    (runtimeGeneration === null && afterSequence !== null) ||
    (runtimeGeneration !== null &&
      (!isSafeUnsignedInteger(runtimeGeneration) ||
        (afterSequence !== null && !isSafeUnsignedInteger(afterSequence)))) ||
    (runtimeGeneration !== null &&
      runtimeGeneration !== batch.backlog.runtimeGeneration) ||
    (afterSequence !== null && afterSequence > batch.backlog.latestSequence)
  ) {
    return false;
  }

  const seenBacklogSequences = new Set<number>();
  for (const event of batch.backlog.events) {
    if (
      !codeScopesEqual(state.scope, event.scope) ||
      event.runtimeGeneration !== batch.backlog.runtimeGeneration ||
      !isSafeUnsignedInteger(event.sequence) ||
      event.sequence > batch.backlog.latestSequence ||
      (afterSequence !== null && event.sequence <= afterSequence) ||
      seenBacklogSequences.has(event.sequence)
    ) {
      return false;
    }
    seenBacklogSequences.add(event.sequence);
  }

  const checkpoint = batch.backlog.checkpoint;
  if (checkpoint !== null) {
    if (
      checkpoint.runtimeGeneration !== batch.backlog.runtimeGeneration ||
      checkpoint.sequenceWatermark !== batch.backlog.latestSequence
    ) {
      return false;
    }
    const activeTurnKeys = new Set<string>();
    for (const turn of checkpoint.activeTurns) {
      const key = activeTurnKey(turn.threadId, turn.turnId);
      if (
        turn.threadId.length === 0 ||
        turn.turnId.length === 0 ||
        turn.status.length === 0 ||
        !isSafeUnsignedInteger(turn.startedSequence) ||
        turn.startedSequence > checkpoint.sequenceWatermark ||
        activeTurnKeys.has(key)
      ) {
        return false;
      }
      activeTurnKeys.add(key);
    }
    const approvalKeys = new Set<string>();
    for (const approval of checkpoint.pendingApprovals) {
      const pending = pendingApprovalFromEvent(
        approval.event,
        approval.respondable,
      );
      if (
        pending === null ||
        !codeScopesEqual(state.scope, approval.event.scope) ||
        approval.event.runtimeGeneration !== checkpoint.runtimeGeneration ||
        approval.event.sequence !== checkpoint.sequenceWatermark
      ) {
        return false;
      }
      const key = codeApprovalIdentityKey(pending);
      if (approvalKeys.has(key)) return false;
      approvalKeys.add(key);
    }
  }

  return batch.bufferedEvents.every(
    (event) =>
      codeScopesEqual(state.scope, event.scope) &&
      event.runtimeGeneration === batch.backlog.runtimeGeneration &&
      isSafeUnsignedInteger(event.runtimeGeneration) &&
      isSafeUnsignedInteger(event.sequence) &&
      (afterSequence === null || event.sequence > afterSequence),
  );
}

function applyCheckpoint(
  state: CodeSessionState,
  checkpoint: NonNullable<CodeWorkspaceReplayBatch["backlog"]["checkpoint"]>,
): CodeSessionState {
  const activeTurns = new Map<string, CodeActiveTurn>();
  for (const turn of checkpoint.activeTurns) {
    activeTurns.set(activeTurnKey(turn.threadId, turn.turnId), {
      runtimeGeneration: checkpoint.runtimeGeneration,
      threadId: turn.threadId,
      turnId: turn.turnId,
      status: turn.status,
      startedSequence: turn.startedSequence,
    });
  }
  const pendingApprovals = new Map<string, CodePendingApproval>();
  for (const approval of checkpoint.pendingApprovals) {
    const pending = pendingApprovalFromEvent(
      approval.event,
      approval.respondable,
    );
    if (pending !== null) {
      pendingApprovals.set(codeApprovalIdentityKey(pending), pending);
    }
  }
  return {
    ...state,
    latestSequence: Math.max(
      state.latestSequence,
      checkpoint.sequenceWatermark,
    ),
    activeTurns,
    pendingApprovals,
  };
}

function markReplayInvalid(state: CodeSessionState): CodeSessionState {
  return {
    ...state,
    replay: {
      ...state.replay,
      status: "invalid",
      needsAuthoritativeRefresh: true,
      approvalStateIncomplete: true,
    },
    activeTurns: new Map(),
    pendingApprovals: new Map(),
  };
}

function applyReplay(
  state: CodeSessionState,
  batch: CodeWorkspaceReplayBatch,
): CodeSessionState {
  if (state.replay.subscriptionEpoch !== batch.subscriptionEpoch) return state;
  if (!replayBatchIsValid(state, batch)) return markReplayInvalid(state);

  const { backlog } = batch;
  let next = state;
  let generationChanged = false;
  if (
    next.runtimeGeneration === null ||
    backlog.runtimeGeneration > next.runtimeGeneration
  ) {
    generationChanged =
      next.runtimeGeneration !== null &&
      backlog.runtimeGeneration > next.runtimeGeneration;
    next = resetForGeneration(next, backlog.runtimeGeneration);
  } else if (backlog.runtimeGeneration < next.runtimeGeneration) {
    return [...batch.bufferedEvents]
      .sort(compareEvents)
      .reduce(
        (current, event) => applyEvent(current, event, batch.subscriptionEpoch),
        next,
      );
  }

  const isFullReplay =
    batch.request.afterSequence === null || batch.request.afterSequence === 0;
  const hasAuthoritativeTransientState =
    backlog.checkpoint !== null || (isFullReplay && !backlog.truncated);
  const healsIncompleteReplay =
    hasAuthoritativeTransientState &&
    !batch.bufferTruncated &&
    state.replay.approvalStateIncomplete;
  const alreadyIncomplete =
    !generationChanged &&
    state.replay.approvalStateIncomplete &&
    !healsIncompleteReplay;
  const newlyIncomplete =
    batch.bufferTruncated || (backlog.truncated && backlog.checkpoint === null);
  if (
    backlog.truncated ||
    newlyIncomplete ||
    healsIncompleteReplay ||
    isFullReplay
  ) {
    next = {
      ...next,
      latestSequence: 0,
      events: [],
      activeTurns: new Map(),
      pendingApprovals: new Map(),
    };
  }

  const replayEvents = new Map<number, CodeWorkspaceEvent>();
  for (const event of batch.backlog.events) {
    replayEvents.set(event.sequence, event);
  }
  for (const event of [...replayEvents.values()].sort(compareEvents)) {
    next = applyEvent(next, event, batch.subscriptionEpoch);
  }
  if (backlog.checkpoint !== null) {
    next = applyCheckpoint(next, backlog.checkpoint);
  }
  const bufferedEvents = new Map<number, CodeWorkspaceEvent>();
  for (const event of batch.bufferedEvents) {
    if (event.runtimeGeneration === backlog.runtimeGeneration) {
      bufferedEvents.set(event.sequence, event);
    }
  }
  for (const event of [...bufferedEvents.values()].sort(compareEvents)) {
    next = applyEvent(next, event, batch.subscriptionEpoch);
  }

  if (next.runtimeGeneration === backlog.runtimeGeneration) {
    const incomplete = alreadyIncomplete || newlyIncomplete;
    const previousIncompleteStatus =
      state.replay.status === "invalid" && !healsIncompleteReplay
        ? "invalid"
        : "truncated";
    next = {
      ...next,
      latestSequence: Math.max(next.latestSequence, backlog.latestSequence),
      replay: incomplete
        ? {
            status: newlyIncomplete ? "truncated" : previousIncompleteStatus,
            subscriptionEpoch: batch.subscriptionEpoch,
            request: batch.request,
            needsAuthoritativeRefresh: backlog.truncated
              ? true
              : generationChanged
                ? false
                : state.replay.needsAuthoritativeRefresh,
            approvalStateIncomplete: true,
          }
        : {
            status: "synchronized",
            subscriptionEpoch: batch.subscriptionEpoch,
            request: batch.request,
            needsAuthoritativeRefresh: backlog.truncated,
            approvalStateIncomplete: false,
          },
    };
  }

  return next;
}

function applyRuntimeStatus(
  state: CodeSessionState,
  status: CodeRuntimeStatus,
  revision: number,
): CodeSessionState {
  if (
    !isSafeUnsignedInteger(revision) ||
    revision <= state.runtimeStatusRevision
  ) {
    return state;
  }
  if (
    state.runtimeGeneration !== null &&
    status.generation < state.runtimeGeneration
  ) {
    return { ...state, runtimeStatusRevision: revision };
  }

  let next =
    status.generation !== state.runtimeGeneration
      ? resetForGeneration(state, status.generation, status, revision)
      : { ...state, runtimeStatus: status, runtimeStatusRevision: revision };

  if (
    status.phase !== "ready" &&
    (next.activeTurns.size > 0 || next.pendingApprovals.size > 0)
  ) {
    next = {
      ...next,
      activeTurns: new Map(),
      pendingApprovals: new Map(),
    };
  }
  return next;
}

function startSubscription(
  state: CodeSessionState,
  subscriptionEpoch: number,
  input: CodeRuntimeEventsInput,
): CodeSessionState {
  const validCursor =
    (input.runtimeGeneration === null && input.afterSequence === null) ||
    (input.runtimeGeneration !== null &&
      isSafeUnsignedInteger(input.runtimeGeneration) &&
      (input.afterSequence === null ||
        isSafeUnsignedInteger(input.afterSequence)));
  if (
    !isSafeUnsignedInteger(subscriptionEpoch) ||
    !codeScopesEqual(state.scope, input.scope) ||
    !validCursor ||
    (state.replay.subscriptionEpoch !== null &&
      subscriptionEpoch <= state.replay.subscriptionEpoch)
  ) {
    return state;
  }
  return {
    ...state,
    replay: {
      ...state.replay,
      status: "synchronizing",
      subscriptionEpoch,
      request: input,
    },
  };
}

function commitApprovalResponse(
  state: CodeSessionState,
  input: CodeApprovalResponseInput,
  expectedSequence: number,
  expectedItemId: string,
): CodeSessionState {
  if (
    state.runtimeGeneration !== input.runtimeGeneration ||
    !codeScopesEqual(state.scope, input.scope)
  ) {
    return state;
  }
  const key = codeApprovalIdentityKey(input);
  const currentApproval = state.pendingApprovals.get(key);
  if (
    currentApproval?.sequence !== expectedSequence ||
    currentApproval.itemId !== expectedItemId
  ) {
    return state;
  }
  const pendingApprovals = new Map(state.pendingApprovals);
  pendingApprovals.delete(key);
  return { ...state, pendingApprovals };
}

function commitTurnInterrupt(
  state: CodeSessionState,
  runtimeGeneration: number,
  input: CodeTurnInterruptInput,
): CodeSessionState {
  if (
    state.runtimeGeneration !== runtimeGeneration ||
    !codeScopesEqual(state.scope, input.scope)
  ) {
    return state;
  }
  const derived = removeTurnState(
    state.activeTurns,
    state.pendingApprovals,
    input.threadId,
    input.turnId,
  );
  if (
    derived.activeTurns === state.activeTurns &&
    derived.pendingApprovals === state.pendingApprovals
  ) {
    return state;
  }
  return { ...state, ...derived };
}

/** Reduce runtime snapshots, replay batches, and live normalized events. */
export function codeSessionReducer(
  state: CodeSessionState,
  action: CodeSessionAction,
): CodeSessionState {
  switch (action.type) {
    case "runtimeStatusReceived":
      return applyRuntimeStatus(state, action.status, action.revision);
    case "subscriptionStarted":
      return startSubscription(state, action.subscriptionEpoch, action.input);
    case "eventReceived":
      return applyEvent(state, action.event, action.subscriptionEpoch);
    case "replayReceived":
      return applyReplay(state, action.batch);
    case "approvalResponseCommitted":
      return commitApprovalResponse(
        state,
        action.input,
        action.expectedSequence,
        action.expectedItemId,
      );
    case "turnInterruptCommitted":
      return commitTurnInterrupt(state, action.runtimeGeneration, action.input);
    case "authoritativeRefreshCompleted":
      return state.runtimeGeneration === action.runtimeGeneration &&
        state.replay.subscriptionEpoch === action.subscriptionEpoch &&
        state.replay.needsAuthoritativeRefresh
        ? {
            ...state,
            replay: { ...state.replay, needsAuthoritativeRefresh: false },
          }
        : state;
    case "reset":
      return createCodeSessionState(state.scope);
  }
}

/** Build the next native replay cursor; sequence gaps alone are not errors. */
export function selectCodeRuntimeEventsInput(
  state: CodeSessionState,
  forceFullReplay = false,
): CodeRuntimeEventsInput {
  if (state.runtimeGeneration === null) {
    return {
      scope: state.scope,
      runtimeGeneration: null,
      afterSequence: null,
    };
  }
  if (
    forceFullReplay ||
    state.replay.status === "idle" ||
    state.replay.status === "invalid"
  ) {
    return {
      scope: state.scope,
      runtimeGeneration: state.runtimeGeneration,
      afterSequence: 0,
    };
  }
  return {
    scope: state.scope,
    runtimeGeneration: state.runtimeGeneration,
    afterSequence: state.latestSequence,
  };
}

/** Return retained events for one bound thread in sequence order. */
export function selectCodeThreadEvents(
  state: CodeSessionState,
  threadId: string,
): readonly CodeWorkspaceEvent[] {
  return state.events.filter((event) => event.threadId === threadId);
}

/** Return active turns for one thread, ordered by their start sequence. */
export function selectCodeActiveTurns(
  state: CodeSessionState,
  threadId: string,
): CodeActiveTurn[] {
  return [...state.activeTurns.values()]
    .filter((turn) => turn.threadId === threadId)
    .sort((left, right) => left.startedSequence - right.startedSequence);
}

/** Return pending approvals, optionally restricted to one thread. */
export function selectCodePendingApprovals(
  state: CodeSessionState,
  threadId?: string,
): CodePendingApproval[] {
  return [...state.pendingApprovals.values()]
    .filter(
      (approval) => threadId === undefined || approval.threadId === threadId,
    )
    .sort((left, right) => left.sequence - right.sequence);
}

/** Fail closed unless the exact pending approval belongs to the ready generation. */
export function selectCanRespondToCodeApproval(
  state: CodeSessionState,
  approval: CodePendingApproval,
): boolean {
  const currentApproval = state.pendingApprovals.get(
    codeApprovalIdentityKey(approval),
  );
  return (
    state.runtimeStatus?.phase === "ready" &&
    state.runtimeStatus.generation === approval.runtimeGeneration &&
    currentApproval?.sequence === approval.sequence &&
    currentApproval.itemId === approval.itemId &&
    currentApproval.approvalKind === approval.approvalKind &&
    currentApproval.respondable
  );
}
