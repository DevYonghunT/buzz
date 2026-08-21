import assert from "node:assert/strict";
import test from "node:test";

import { projectCodeTimeline } from "./codeTimeline.ts";

const scope = {
  communityId: "community-1",
  projectDtag: "project-1",
  repositoryIdentity: "a".repeat(64),
};

function thread(turns = []) {
  return {
    id: "thread-1",
    sessionId: "session-1",
    forkedFromId: null,
    parentThreadId: null,
    preview: "A Code task",
    ephemeral: false,
    modelProvider: "openai",
    createdAt: 1,
    updatedAt: 2,
    cwd: "/native/stored-root",
    name: "Code task",
    status: { type: "idle" },
    turns,
  };
}

function event({
  sequence,
  kind,
  payload,
  threadId = "thread-1",
  turnId = "turn-live",
  itemId = null,
}) {
  return {
    scope,
    runtimeGeneration: 7,
    sequence,
    threadId,
    turnId,
    itemId,
    kind,
    payload,
  };
}

test("restored agent messages are projected from the thread snapshot", () => {
  const rows = projectCodeTimeline(
    thread([
      {
        id: "turn-restored",
        status: "completed",
        items: [
          {
            id: "item-restored",
            type: "agentMessage",
            text: "Restored answer",
          },
        ],
        error: null,
      },
    ]),
    [],
  );

  assert.deepEqual(
    rows.map((row) => row.kind),
    ["agent", "turnStatus"],
  );
  assert.deepEqual(rows[0], {
    key: JSON.stringify(["item", "turn-restored", "item-restored", "agent"]),
    threadId: "thread-1",
    turnId: "turn-restored",
    itemId: "item-restored",
    firstSequence: null,
    lastSequence: null,
    kind: "agent",
    text: "Restored answer",
    streaming: false,
  });
  assert.equal(rows[1].status, "completed");
});

test("compatible deltas aggregate by item in normalized sequence order", () => {
  const rows = projectCodeTimeline(thread(), [
    event({
      sequence: 4,
      kind: "item/agentMessage/delta",
      itemId: "agent-1",
      payload: { delta: "world" },
    }),
    event({
      sequence: 1,
      kind: "turn/started",
      payload: { turn: { id: "turn-live", status: "inProgress" } },
    }),
    event({
      sequence: 2,
      kind: "item/agentMessage/delta",
      itemId: "agent-1",
      payload: { delta: "Hello " },
    }),
    event({
      sequence: 3,
      kind: "item/agentMessage/delta",
      itemId: "agent-1",
      payload: { delta: "small " },
    }),
    event({
      sequence: 5,
      kind: "item/agentMessage/delta",
      itemId: "agent-2",
      payload: { delta: "Second row" },
    }),
  ]);

  assert.deepEqual(
    rows.map((row) => row.kind),
    ["turnStatus", "agent", "agent"],
  );
  assert.equal(rows[1].text, "Hello small world");
  assert.equal(rows[1].firstSequence, 2);
  assert.equal(rows[1].lastSequence, 4);
  assert.equal(rows[2].text, "Second row");
  assert.equal(Object.hasOwn(rows[1], "payload"), false);
});

test("private reasoning text is discarded while public summaries remain", () => {
  const rows = projectCodeTimeline(thread(), [
    event({
      sequence: 1,
      kind: "item/reasoning/summaryTextDelta",
      itemId: "reasoning-1",
      payload: { delta: "Checking the public contract" },
    }),
    event({
      sequence: 2,
      kind: "item/reasoning/textDelta",
      itemId: "reasoning-1",
      payload: {
        delta: "PRIVATE_REASONING_MUST_NOT_RENDER",
        arbitrary: { secret: "RAW_PAYLOAD_MUST_NOT_RENDER" },
      },
    }),
  ]);

  assert.equal(rows.length, 1);
  assert.equal(rows[0].kind, "plan");
  assert.equal(rows[0].text, "Checking the public contract");
  const serialized = JSON.stringify(rows);
  assert.equal(serialized.includes("PRIVATE_REASONING_MUST_NOT_RENDER"), false);
  assert.equal(serialized.includes("RAW_PAYLOAD_MUST_NOT_RENDER"), false);
  assert.equal(serialized.includes('"payload"'), false);
});

test("plan, command, file, warning, and error events become semantic rows", () => {
  const rows = projectCodeTimeline(thread(), [
    event({
      sequence: 1,
      kind: "turn/plan/updated",
      payload: {
        explanation: "Focused verification",
        plan: [{ step: "Run tests", status: "inProgress" }],
      },
    }),
    event({
      sequence: 2,
      kind: "item/started",
      itemId: "command-1",
      payload: {
        item: {
          id: "command-1",
          type: "commandExecution",
          command: "pnpm test",
          status: "inProgress",
        },
      },
    }),
    event({
      sequence: 3,
      kind: "item/commandExecution/outputDelta",
      itemId: "command-1",
      payload: { delta: "tests " },
    }),
    event({
      sequence: 4,
      kind: "item/commandExecution/outputDelta",
      itemId: "command-1",
      payload: { delta: "passed" },
    }),
    event({
      sequence: 5,
      kind: "item/completed",
      itemId: "command-1",
      payload: {
        item: {
          id: "command-1",
          type: "commandExecution",
          command: "pnpm test",
          aggregatedOutput: "tests passed",
          status: "completed",
          exitCode: 0,
        },
      },
    }),
    event({
      sequence: 6,
      kind: "item/fileChange/patchUpdated",
      itemId: "file-1",
      payload: {
        changes: [
          {
            path: "src/code.ts",
            kind: { type: "update" },
            diff: "+raw patch body stays out of timeline",
          },
        ],
      },
    }),
    event({
      sequence: 7,
      kind: "warning",
      itemId: null,
      payload: { message: "Retrying a read" },
    }),
    event({
      sequence: 8,
      kind: "error",
      itemId: null,
      payload: {
        error: { message: "Command failed", debug: "do not expose this" },
      },
    }),
  ]);

  assert.deepEqual(
    rows.map((row) => row.kind),
    ["plan", "commandOutput", "fileChange", "warning", "error"],
  );
  assert.deepEqual(rows[0], {
    ...rows[0],
    text: "Focused verification",
    steps: [{ text: "Run tests", status: "inProgress" }],
    streaming: false,
  });
  assert.equal(rows[1].command, "pnpm test");
  assert.equal(rows[1].output, "tests passed");
  assert.equal(rows[1].status, "completed");
  assert.equal(rows[1].exitCode, 0);
  assert.equal(rows[1].streaming, false);
  assert.deepEqual(rows[2].changes, [
    { path: "src/code.ts", changeType: "update" },
  ]);
  assert.equal(rows[3].message, "Retrying a read");
  assert.equal(rows[4].message, "Command failed");
  const serialized = JSON.stringify(rows);
  assert.equal(serialized.includes("raw patch body"), false);
  assert.equal(serialized.includes("do not expose this"), false);
});

test("local submitted prompts project as user rows without leaking other threads", () => {
  const rows = projectCodeTimeline(
    thread(),
    [
      event({
        sequence: 1,
        kind: "item/agentMessage/delta",
        threadId: "thread-other",
        itemId: "foreign",
        payload: { delta: "foreign output" },
      }),
    ],
    [{ id: "prompt-1", text: "Please update the tests" }],
  );

  assert.equal(rows.length, 1);
  assert.equal(rows[0].kind, "user");
  assert.equal(rows[0].text, "Please update the tests");
  assert.equal(rows[0].pending, true);
  assert.equal(JSON.stringify(rows).includes("foreign output"), false);
});

test("persisted user messages reconcile only exact optimistic prompts", () => {
  const rows = projectCodeTimeline(
    thread([
      {
        id: "turn-restored",
        status: "inProgress",
        items: [
          {
            id: "persisted-prompt",
            type: "userMessage",
            text: "Repeat this prompt",
          },
        ],
        error: null,
      },
    ]),
    [],
    [
      {
        id: "local-matched",
        text: "Repeat this prompt",
        turnId: "turn-restored",
      },
      {
        id: "local-still-waiting",
        text: "Repeat this prompt",
        turnId: "turn-restored",
      },
      {
        id: "local-different-text",
        text: "Different prompt",
        turnId: "turn-restored",
      },
      {
        id: "local-different-turn",
        text: "Repeat this prompt",
        turnId: "turn-pending",
      },
      {
        id: "local-without-turn",
        text: "Repeat this prompt",
      },
    ],
  );
  const userRows = rows.filter((row) => row.kind === "user");

  assert.deepEqual(
    userRows.map((row) => row.itemId),
    [
      "persisted-prompt",
      "local-still-waiting",
      "local-different-text",
      "local-different-turn",
      "local-without-turn",
    ],
  );
  assert.equal(
    userRows.some((row) => row.itemId === "local-matched"),
    false,
  );
});
