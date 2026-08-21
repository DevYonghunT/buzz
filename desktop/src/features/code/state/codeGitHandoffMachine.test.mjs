import assert from "node:assert/strict";
import test from "node:test";

import {
  CODE_GIT_RECONCILE_DELAYS_MS,
  codeGitReconcileReceiptError,
  pollCodeGitReconcile,
  settleCodeGitReceipt,
} from "./codeGitHandoffMachine.ts";

const scope = {
  communityId: "community-1",
  projectDtag: "project-1",
  repositoryIdentity: "a".repeat(64),
};
const receipt = {
  operationId: "b".repeat(64),
  operation: "commit",
  scope,
  threadId: "thread-1",
  requestGeneration: 4,
  beforeSnapshotId: "c".repeat(64),
  previousHead: "d".repeat(40),
  commit: "e".repeat(40),
  tree: "f".repeat(40),
  disposition: "committed",
};

function ready({ blockingReceipt, statusRevision, writeGeneration }) {
  return {
    state: "ready",
    runtimeGeneration: 7,
    statusRevision,
    writeGeneration,
    snapshotSequence: writeGeneration,
    scope,
    threadId: "thread-1",
    snapshotId: "1".repeat(64),
    headCommit: receipt.commit,
    task: { files: [], totalFiles: 0, filesTruncated: false },
    staged: { files: [], totalFiles: 0, filesTruncated: false },
    unstaged: { files: [], totalFiles: 0, filesTruncated: false },
    hasConflicts: false,
    commitIdentity: null,
    capabilities: {
      stage: { enabled: true, reason: null },
      unstage: { enabled: true, reason: null },
      commit: { enabled: true, reason: null },
    },
    blockingReceipt,
  };
}

test("completed receipt settles from fresh revisions without previous component cache", async () => {
  const statuses = [
    ready({ blockingReceipt: receipt, statusRevision: 11, writeGeneration: 5 }),
    ready({ blockingReceipt: null, statusRevision: 12, writeGeneration: 5 }),
  ];
  const events = [];
  let acknowledgements = 0;

  const result = await settleCodeGitReceipt({
    acceptStatus: (status) =>
      events.push(
        status.blockingReceipt ? "accept-blocking" : "accept-cleared",
      ),
    receipt,
    minimumStatusRevision: 10,
    readStatus: async () => {
      events.push("status");
      return statuses.shift();
    },
    acknowledge: async (blocking) => {
      events.push("acknowledge");
      acknowledgements += 1;
      assert.equal(blocking.blockingReceipt, receipt);
    },
  });

  assert.equal(result.state, "settled");
  assert.equal(result.acknowledgementResponseLost, false);
  assert.equal(acknowledgements, 1);
  assert.equal(statuses.length, 0);
  assert.deepEqual(events, [
    "status",
    "accept-blocking",
    "acknowledge",
    "status",
    "accept-cleared",
  ]);
});

test("cached exact blocker settles without requiring an unavailable old baseline", async () => {
  const statuses = [
    ready({ blockingReceipt: receipt, statusRevision: 11, writeGeneration: 5 }),
    ready({ blockingReceipt: null, statusRevision: 12, writeGeneration: 5 }),
  ];

  const result = await settleCodeGitReceipt({
    acceptStatus() {},
    receipt,
    minimumStatusRevision: null,
    readStatus: async () => statuses.shift(),
    acknowledge: async () => {},
  });

  assert.equal(result.state, "settled");
});

test("semantically wrong mutation status is rejected before cache acceptance", async () => {
  const accepted = [];
  let acknowledgements = 0;
  const result = await settleCodeGitReceipt({
    acceptStatus: (status) => accepted.push(status),
    receipt,
    minimumStatusRevision: 10,
    readStatus: async () =>
      ready({ blockingReceipt: null, statusRevision: 11, writeGeneration: 5 }),
    acknowledge: async () => {
      acknowledgements += 1;
    },
  });

  assert.equal(result.state, "unknown");
  assert.deepEqual(accepted, []);
  assert.equal(acknowledgements, 0);
});

test("completed reconcile requires the exact request generation and coordinate", () => {
  assert.equal(
    codeGitReconcileReceiptError({
      expectedOperation: "commit",
      expectedOperationId: receipt.operationId,
      expectedRequestGeneration: receipt.requestGeneration,
      receipt,
    }),
    null,
  );
  assert.match(
    codeGitReconcileReceiptError({
      expectedOperation: "commit",
      expectedOperationId: receipt.operationId,
      expectedRequestGeneration: receipt.requestGeneration + 1,
      receipt,
    }),
    /write generation/,
  );
});

test("acknowledgement response loss settles only after fresh cleared status", async () => {
  const statuses = [
    ready({ blockingReceipt: receipt, statusRevision: 11, writeGeneration: 5 }),
    ready({ blockingReceipt: null, statusRevision: 12, writeGeneration: 5 }),
  ];

  const result = await settleCodeGitReceipt({
    acceptStatus() {},
    receipt,
    minimumStatusRevision: 10,
    readStatus: async () => statuses.shift(),
    acknowledge: async () => {
      throw new Error("transport closed after acknowledgement");
    },
  });

  assert.equal(result.state, "settled");
  assert.equal(result.acknowledgementResponseLost, true);
});

test("acknowledgement response loss retains an unchanged blocker", async () => {
  const blocking = ready({
    blockingReceipt: receipt,
    statusRevision: 11,
    writeGeneration: 5,
  });
  const result = await settleCodeGitReceipt({
    acceptStatus() {},
    receipt,
    minimumStatusRevision: 10,
    readStatus: async () => blocking,
    acknowledge: async () => {
      throw new Error("transport closed after acknowledgement");
    },
  });

  assert.equal(result.state, "unknown");
  assert.match(result.message, /still blocking writes/);
});

test("pending recovery polling is bounded before yielding explicit retry", async () => {
  const waits = [];
  let calls = 0;
  const result = await pollCodeGitReconcile({
    reconcile: async () => {
      calls += 1;
      return {
        state: calls % 2 === 0 ? "recovering" : "pending",
        operation: "commit",
        operationId: receipt.operationId,
        scope,
        threadId: receipt.threadId,
      };
    },
    onProgress() {},
    wait: async (milliseconds) => waits.push(milliseconds),
  });

  assert.deepEqual(waits, [...CODE_GIT_RECONCILE_DELAYS_MS]);
  assert.equal(calls, CODE_GIT_RECONCILE_DELAYS_MS.length + 1);
  assert.deepEqual(result, {
    state: "exhausted",
    operation: "commit",
    operationId: receipt.operationId,
  });
});
