import assert from "node:assert/strict";
import test from "node:test";

import {
  codeApprovalIdentityKey,
  codeSessionReducer,
  createCodeSessionState,
  selectCanRespondToCodeApproval,
  selectCodeActiveTurns,
  selectCodePendingApprovals,
  selectCodeRuntimeEventsInput,
  selectCodeThreadEvents,
} from "./codeSessionReducer.ts";

const scope = {
  communityId: "community-1",
  projectDtag: "project-1",
  repositoryIdentity: "a".repeat(64),
};
const epoch = 1;

const readyStatus = (generation = 7) => ({
  phase: "ready",
  generation,
  executable: "/usr/local/bin/codex",
  version: "codex-cli 0.145.0",
  pid: 1234,
  userAgent: "codex-test",
  codexHome: "/native/codex-home",
  platformFamily: "unix",
  platformOs: "macos",
  queuedNotifications: 0,
  lastError: null,
});

function event({
  sequence,
  generation = 7,
  kind = "item/agentMessage/delta",
  eventScope = scope,
  threadId = "thread-1",
  turnId = "turn-1",
  itemId = "item-1",
  payload = { delta: `event-${sequence}` },
}) {
  return {
    scope: eventScope,
    runtimeGeneration: generation,
    sequence,
    threadId,
    turnId,
    itemId,
    kind,
    payload,
  };
}

function approvalEvent({
  sequence,
  requestId,
  generation = 7,
  kind = "item/commandExecution/requestApproval",
  approvalKind = "commandExecution",
  requestOverrides = {},
}) {
  const itemId = `approval-item-${String(requestId)}`;
  return event({
    sequence,
    generation,
    kind,
    itemId,
    payload: {
      requestId,
      approvalKind,
      request: {
        threadId: "thread-1",
        turnId: "turn-1",
        itemId,
        command: "cargo test",
        ...requestOverrides,
      },
    },
  });
}

function replayInput(generation = 7, afterSequence = 0) {
  return { scope, runtimeGeneration: generation, afterSequence };
}

function replayBatch({
  generation = 7,
  latestSequence,
  truncated = false,
  checkpoint = null,
  events = [],
  bufferedEvents = [],
  bufferTruncated = false,
  subscriptionEpoch = epoch,
  request = replayInput(generation, 0),
}) {
  return {
    subscriptionEpoch,
    request,
    backlog: {
      runtimeGeneration: generation,
      latestSequence,
      truncated,
      checkpoint,
      events,
    },
    bufferedEvents,
    bufferTruncated,
  };
}

function withReadyRuntime(state, generation = 7, revision = 1) {
  return codeSessionReducer(state, {
    type: "runtimeStatusReceived",
    revision,
    status: readyStatus(generation),
  });
}

function withSubscription(state, subscriptionEpoch = epoch, input) {
  return codeSessionReducer(state, {
    type: "subscriptionStarted",
    subscriptionEpoch,
    input: input ?? replayInput(state.runtimeGeneration ?? 7, 0),
  });
}

function receive(state, incoming, subscriptionEpoch = epoch) {
  return codeSessionReducer(state, {
    type: "eventReceived",
    subscriptionEpoch,
    event: incoming,
  });
}

function readySubscribedState() {
  return withSubscription(withReadyRuntime(createCodeSessionState(scope)));
}

test("replay merges buffered live events and rejects stale sequence or epoch", () => {
  const initial = readySubscribedState();
  const replayed = codeSessionReducer(initial, {
    type: "replayReceived",
    batch: replayBatch({
      latestSequence: 11,
      events: [event({ sequence: 11 })],
      bufferedEvents: [event({ sequence: 13 }), event({ sequence: 12 })],
    }),
  });

  assert.deepEqual(
    replayed.events.map(({ sequence }) => sequence),
    [11, 12, 13],
  );
  assert.equal(replayed.latestSequence, 13);
  assert.deepEqual(replayed.replay, {
    status: "synchronized",
    subscriptionEpoch: epoch,
    request: replayInput(),
    needsAuthoritativeRefresh: false,
    approvalStateIncomplete: false,
  });

  assert.equal(receive(replayed, event({ sequence: 13 })), replayed);
  assert.equal(receive(replayed, event({ sequence: 10 })), replayed);
  assert.equal(receive(replayed, event({ sequence: 14 }), epoch - 1), replayed);

  const second = withSubscription(replayed, 2, replayInput(7, 13));
  assert.equal(
    codeSessionReducer(second, {
      type: "replayReceived",
      batch: replayBatch({
        latestSequence: 14,
        events: [event({ sequence: 14 })],
      }),
    }),
    second,
  );
});

test("global sequence jumps are accepted without inventing a replay gap", () => {
  let state = readySubscribedState();
  state = codeSessionReducer(state, {
    type: "replayReceived",
    batch: replayBatch({ latestSequence: 4 }),
  });
  state = receive(state, event({ sequence: 20 }));
  assert.equal(state.latestSequence, 20);
  assert.equal(state.replay.status, "synchronized");

  state = codeSessionReducer(state, {
    type: "replayReceived",
    batch: replayBatch({
      latestSequence: 30,
      request: replayInput(7, 20),
    }),
  });
  assert.deepEqual(selectCodeRuntimeEventsInput(state), replayInput(7, 30));
});

test("exact scope and generation boundaries reset or reject state", () => {
  const initial = readySubscribedState();
  assert.equal(
    receive(
      initial,
      event({
        sequence: 1,
        eventScope: { ...scope, projectDtag: "other-project" },
      }),
    ),
    initial,
  );

  let current = receive(
    initial,
    event({
      sequence: 1,
      kind: "turn/started",
      payload: { turn: { id: "turn-1", status: "inProgress" } },
    }),
  );
  current = receive(
    current,
    approvalEvent({ sequence: 2, requestId: "approval-1" }),
  );
  assert.equal(current.activeTurns.size, 1);
  assert.equal(current.pendingApprovals.size, 1);
  assert.equal(
    receive(current, event({ sequence: 999, generation: 6 })),
    current,
  );

  const restarted = codeSessionReducer(current, {
    type: "runtimeStatusReceived",
    revision: 2,
    status: readyStatus(8),
  });
  assert.equal(restarted.runtimeGeneration, 8);
  assert.equal(restarted.latestSequence, 0);
  assert.equal(restarted.events.length, 0);
  assert.equal(restarted.activeTurns.size, 0);
  assert.equal(restarted.pendingApprovals.size, 0);
});

test("numeric and string approval ids remain distinct and resolve exactly", () => {
  let state = readySubscribedState();
  state = receive(
    state,
    event({
      sequence: 1,
      kind: "turn/started",
      payload: { turn: { id: "turn-1", status: "inProgress" } },
    }),
  );
  state = receive(state, approvalEvent({ sequence: 2, requestId: 9 }));
  state = receive(state, approvalEvent({ sequence: 3, requestId: "9" }));

  const approvals = selectCodePendingApprovals(state);
  assert.equal(approvals.length, 2);
  assert.notEqual(
    codeApprovalIdentityKey(approvals[0]),
    codeApprovalIdentityKey(approvals[1]),
  );
  assert.equal(selectCanRespondToCodeApproval(state, approvals[0]), true);

  state = receive(
    state,
    event({
      sequence: 4,
      kind: "serverRequest/resolved",
      itemId: null,
      turnId: null,
      payload: { threadId: "thread-1", requestId: 9 },
    }),
  );
  assert.deepEqual(
    selectCodePendingApprovals(state).map(({ requestId }) => requestId),
    ["9"],
  );

  state = receive(
    state,
    event({
      sequence: 5,
      kind: "turn/completed",
      payload: { turn: { id: "turn-1", status: "completed" } },
    }),
  );
  assert.equal(state.pendingApprovals.size, 0);
  assert.equal(state.activeTurns.size, 0);
});

test("malformed approval identities are never respondable", () => {
  let state = readySubscribedState();
  state = receive(
    state,
    approvalEvent({
      sequence: 1,
      requestId: "mismatch",
      requestOverrides: { turnId: "different-turn" },
    }),
  );
  state = receive(
    state,
    approvalEvent({
      sequence: 2,
      requestId: Number.MAX_SAFE_INTEGER + 1,
    }),
  );
  assert.equal(state.pendingApprovals.size, 0);
});

test("a stale approval card cannot authorize a reused request id", () => {
  let state = readySubscribedState();
  state = receive(
    state,
    approvalEvent({ sequence: 1, requestId: "reused-request" }),
  );
  const staleApproval = selectCodePendingApprovals(state)[0];
  state = receive(
    state,
    event({
      sequence: 2,
      kind: "serverRequest/resolved",
      itemId: null,
      turnId: null,
      payload: { threadId: "thread-1", requestId: "reused-request" },
    }),
  );
  const replacement = approvalEvent({
    sequence: 3,
    requestId: "reused-request",
  });
  replacement.itemId = "replacement-item";
  replacement.payload.request.itemId = "replacement-item";
  state = receive(state, replacement);

  assert.equal(selectCanRespondToCodeApproval(state, staleApproval), false);
  assert.equal(
    selectCanRespondToCodeApproval(state, selectCodePendingApprovals(state)[0]),
    true,
  );
});

test("successful approval and interrupt actions consume exact generation identities", () => {
  let state = readySubscribedState();
  state = receive(
    state,
    event({
      sequence: 1,
      kind: "turn/started",
      payload: { turn: { id: "turn-1", status: "inProgress" } },
    }),
  );
  state = receive(
    state,
    approvalEvent({ sequence: 2, requestId: "approval-1" }),
  );

  const wrongGeneration = codeSessionReducer(state, {
    type: "approvalResponseCommitted",
    expectedSequence: 2,
    expectedItemId: "approval-item-approval-1",
    input: {
      runtimeGeneration: 6,
      requestId: "approval-1",
      scope,
      threadId: "thread-1",
      turnId: "turn-1",
      response: { type: "decision", decision: "decline" },
    },
  });
  assert.equal(wrongGeneration, state);

  state = codeSessionReducer(state, {
    type: "approvalResponseCommitted",
    expectedSequence: 2,
    expectedItemId: "approval-item-approval-1",
    input: {
      runtimeGeneration: 7,
      requestId: "approval-1",
      scope,
      threadId: "thread-1",
      turnId: "turn-1",
      response: { type: "decision", decision: "decline" },
    },
  });
  assert.equal(state.pendingApprovals.size, 0);
  assert.equal(state.activeTurns.size, 1);

  assert.equal(
    codeSessionReducer(state, {
      type: "turnInterruptCommitted",
      runtimeGeneration: 6,
      input: { scope, threadId: "thread-1", turnId: "turn-1" },
    }),
    state,
  );
  state = codeSessionReducer(state, {
    type: "turnInterruptCommitted",
    runtimeGeneration: 7,
    input: { scope, threadId: "thread-1", turnId: "turn-1" },
  });
  assert.equal(state.activeTurns.size, 0);
});

test("a late approval commit cannot remove a reused replacement request", () => {
  let state = readySubscribedState();
  state = receive(
    state,
    approvalEvent({ sequence: 1, requestId: "reused-request" }),
  );
  state = receive(
    state,
    event({
      sequence: 2,
      kind: "serverRequest/resolved",
      itemId: null,
      turnId: null,
      payload: { threadId: "thread-1", requestId: "reused-request" },
    }),
  );
  const replacement = approvalEvent({
    sequence: 3,
    requestId: "reused-request",
  });
  replacement.itemId = "replacement-item";
  replacement.payload.request.itemId = "replacement-item";
  state = receive(state, replacement);

  const unchanged = codeSessionReducer(state, {
    type: "approvalResponseCommitted",
    expectedSequence: 1,
    expectedItemId: "approval-item-reused-request",
    input: {
      runtimeGeneration: 7,
      requestId: "reused-request",
      scope,
      threadId: "thread-1",
      turnId: "turn-1",
      response: { type: "decision", decision: "accept" },
    },
  });
  assert.equal(unchanged, state);
  assert.equal(unchanged.pendingApprovals.size, 1);
});

test("truncation is sticky across incremental replay and avoids live-state loss", () => {
  let state = readySubscribedState();
  state = receive(
    state,
    approvalEvent({ sequence: 10, requestId: "already-observed" }),
  );
  state = codeSessionReducer(state, {
    type: "replayReceived",
    batch: replayBatch({
      latestSequence: 10,
      truncated: true,
      events: [approvalEvent({ sequence: 9, requestId: "replayed" })],
      bufferedEvents: [
        approvalEvent({ sequence: 10, requestId: "already-observed" }),
        approvalEvent({ sequence: 11, requestId: "new-live" }),
      ],
    }),
  });

  assert.deepEqual(
    selectCodePendingApprovals(state).map(({ requestId }) => requestId),
    ["replayed", "already-observed", "new-live"],
  );
  assert.equal(state.replay.status, "truncated");
  assert.equal(state.replay.needsAuthoritativeRefresh, true);
  assert.equal(state.replay.approvalStateIncomplete, true);

  state = codeSessionReducer(state, {
    type: "replayReceived",
    batch: replayBatch({
      latestSequence: 11,
      request: replayInput(7, 11),
    }),
  });
  assert.equal(state.replay.status, "truncated");
  assert.equal(state.replay.approvalStateIncomplete, true);
  assert.deepEqual(
    selectCodeRuntimeEventsInput(state, true),
    replayInput(7, 0),
  );

  const staleCompletion = codeSessionReducer(state, {
    type: "authoritativeRefreshCompleted",
    runtimeGeneration: 6,
    subscriptionEpoch: epoch,
  });
  assert.equal(staleCompletion, state);
  state = codeSessionReducer(state, {
    type: "authoritativeRefreshCompleted",
    runtimeGeneration: 7,
    subscriptionEpoch: epoch,
  });
  assert.equal(state.replay.needsAuthoritativeRefresh, false);
  assert.equal(state.replay.approvalStateIncomplete, true);
});

test("authoritative checkpoint heals evicted transient state before buffered live events", () => {
  const checkpointApproval = approvalEvent({
    sequence: 513,
    requestId: "checkpoint-approval",
  });
  const bufferedApproval = approvalEvent({
    sequence: 514,
    requestId: "buffered-approval",
  });
  let state = readySubscribedState();
  state = codeSessionReducer(state, {
    type: "replayReceived",
    batch: replayBatch({
      latestSequence: 513,
      truncated: true,
      checkpoint: {
        runtimeGeneration: 7,
        sequenceWatermark: 513,
        activeTurns: [
          {
            threadId: "thread-1",
            turnId: "turn-1",
            status: "inProgress",
            startedSequence: 1,
          },
        ],
        pendingApprovals: [{ event: checkpointApproval, respondable: true }],
      },
      events: [event({ sequence: 513 })],
      bufferedEvents: [bufferedApproval],
    }),
  });

  assert.equal(state.replay.status, "synchronized");
  assert.equal(state.replay.approvalStateIncomplete, false);
  assert.equal(state.replay.needsAuthoritativeRefresh, true);
  assert.equal(state.latestSequence, 514);
  assert.equal(selectCodeActiveTurns(state, "thread-1").length, 1);
  assert.deepEqual(
    selectCodePendingApprovals(state).map(({ requestId }) => requestId),
    ["checkpoint-approval", "buffered-approval"],
  );

  state = receive(
    state,
    event({
      sequence: 515,
      kind: "serverRequest/resolved",
      itemId: null,
      payload: { requestId: "checkpoint-approval" },
    }),
  );
  assert.deepEqual(
    selectCodePendingApprovals(state).map(({ requestId }) => requestId),
    ["buffered-approval"],
  );
  state = codeSessionReducer(state, {
    type: "authoritativeRefreshCompleted",
    runtimeGeneration: 7,
    subscriptionEpoch: epoch,
  });
  assert.equal(state.replay.needsAuthoritativeRefresh, false);
  assert.equal(
    selectCanRespondToCodeApproval(state, selectCodePendingApprovals(state)[0]),
    true,
  );
});

test("buffer overflow and contradictory replay batches fail closed", () => {
  let state = readySubscribedState();
  state = codeSessionReducer(state, {
    type: "replayReceived",
    batch: replayBatch({ latestSequence: 4, bufferTruncated: true }),
  });
  assert.equal(state.replay.status, "truncated");
  assert.equal(state.replay.approvalStateIncomplete, true);

  const second = withSubscription(state, 2, replayInput(7, 4));
  const invalid = codeSessionReducer(second, {
    type: "replayReceived",
    batch: replayBatch({
      subscriptionEpoch: 2,
      request: replayInput(7, 4),
      latestSequence: 5,
      events: [event({ sequence: 6 })],
    }),
  });
  assert.equal(invalid.replay.status, "invalid");
  assert.equal(invalid.pendingApprovals.size, 0);
  assert.deepEqual(selectCodeRuntimeEventsInput(invalid), replayInput(7, 0));

  const retrying = withSubscription(invalid, 3, replayInput(7, 0));
  const healed = codeSessionReducer(retrying, {
    type: "replayReceived",
    batch: replayBatch({
      subscriptionEpoch: 3,
      request: replayInput(7, 0),
      latestSequence: 1,
      events: [event({ sequence: 1 })],
    }),
  });
  assert.equal(healed.replay.status, "synchronized");
  assert.equal(healed.replay.approvalStateIncomplete, false);

  const nullableCursorRetry = withSubscription(
    invalid,
    4,
    replayInput(7, null),
  );
  const nullableCursorHealed = codeSessionReducer(nullableCursorRetry, {
    type: "replayReceived",
    batch: replayBatch({
      subscriptionEpoch: 4,
      request: replayInput(7, null),
      latestSequence: 1,
      events: [event({ sequence: 1 })],
    }),
  });
  assert.equal(nullableCursorHealed.replay.status, "synchronized");
});

test("pre-cursor buffered duplicates make a truncated replay invalid", () => {
  let state = readySubscribedState();
  state = receive(
    state,
    approvalEvent({ sequence: 5, requestId: "already-resolved" }),
  );
  state = receive(
    state,
    event({
      sequence: 6,
      kind: "serverRequest/resolved",
      itemId: null,
      turnId: null,
      payload: { threadId: "thread-1", requestId: "already-resolved" },
    }),
  );
  state = receive(state, event({ sequence: 10 }));
  state = withSubscription(state, 2, replayInput(7, 10));
  state = codeSessionReducer(state, {
    type: "replayReceived",
    batch: replayBatch({
      subscriptionEpoch: 2,
      request: replayInput(7, 10),
      latestSequence: 10,
      truncated: true,
      bufferedEvents: [
        approvalEvent({ sequence: 5, requestId: "already-resolved" }),
      ],
    }),
  });
  assert.equal(state.replay.status, "invalid");
  assert.equal(state.pendingApprovals.size, 0);
});

test("mixed-generation buffered replay batches fail closed", () => {
  let state = readySubscribedState();
  state = codeSessionReducer(state, {
    type: "replayReceived",
    batch: replayBatch({
      latestSequence: 1,
      bufferedEvents: [event({ sequence: 1, generation: 8 })],
    }),
  });
  assert.equal(state.runtimeGeneration, 7);
  assert.equal(state.replay.status, "invalid");
  assert.equal(state.events.length, 0);
});

test("a complete new-generation replay does not inherit an old gap", () => {
  let state = readySubscribedState();
  state = codeSessionReducer(state, {
    type: "replayReceived",
    batch: replayBatch({ latestSequence: 4, truncated: true }),
  });
  state = withSubscription(state, 2, replayInput(8, 0));
  state = codeSessionReducer(state, {
    type: "replayReceived",
    batch: replayBatch({
      subscriptionEpoch: 2,
      request: replayInput(8, 0),
      generation: 8,
      latestSequence: 1,
      events: [event({ sequence: 1, generation: 8 })],
    }),
  });
  assert.equal(state.runtimeGeneration, 8);
  assert.equal(state.replay.status, "synchronized");
  assert.equal(state.replay.approvalStateIncomplete, false);
});

test("a full replay rebuilds a prefix observed after generation rollover", () => {
  let state = readySubscribedState();
  state = receive(state, event({ sequence: 5, generation: 8 }));
  assert.equal(state.runtimeGeneration, 8);
  assert.equal(state.replay.status, "idle");
  state = withSubscription(state, 2, replayInput(8, 0));
  state = codeSessionReducer(state, {
    type: "replayReceived",
    batch: replayBatch({
      subscriptionEpoch: 2,
      request: replayInput(8, 0),
      generation: 8,
      latestSequence: 5,
      events: [1, 2, 3, 4, 5].map((sequence) =>
        event({ sequence, generation: 8 }),
      ),
    }),
  });
  assert.deepEqual(
    state.events.map(({ sequence }) => sequence),
    [1, 2, 3, 4, 5],
  );
});

test("selectors expose thread-local events and active turns", () => {
  let state = readySubscribedState();
  state = receive(
    state,
    event({
      sequence: 1,
      kind: "turn/started",
      payload: { turn: { id: "turn-1", status: "inProgress" } },
    }),
  );
  state = receive(
    state,
    event({ sequence: 2, threadId: "thread-2", turnId: "turn-2" }),
  );
  assert.deepEqual(
    selectCodeThreadEvents(state, "thread-1").map(({ sequence }) => sequence),
    [1],
  );
  assert.deepEqual(selectCodeActiveTurns(state, "thread-1"), [
    {
      runtimeGeneration: 7,
      threadId: "thread-1",
      turnId: "turn-1",
      status: "inProgress",
      startedSequence: 1,
    },
  ]);
});

test("runtime revisions reject stale status and non-ready state blocks derivation", () => {
  let state = readySubscribedState();
  state = receive(
    state,
    approvalEvent({ sequence: 1, requestId: "approval-1" }),
  );
  state = codeSessionReducer(state, {
    type: "runtimeStatusReceived",
    revision: 2,
    status: { ...readyStatus(), phase: "stopped", pid: null },
  });
  assert.equal(state.pendingApprovals.size, 0);

  const staleReady = codeSessionReducer(state, {
    type: "runtimeStatusReceived",
    revision: 1,
    status: readyStatus(),
  });
  assert.equal(staleReady, state);

  state = receive(
    state,
    approvalEvent({ sequence: 2, requestId: "queued-after-stop" }),
  );
  assert.equal(state.pendingApprovals.size, 0);
});

test("separate states never share mutable replay objects", () => {
  const left = createCodeSessionState(scope);
  const right = createCodeSessionState({ ...scope, projectDtag: "project-2" });
  assert.notEqual(left.replay, right.replay);
});
