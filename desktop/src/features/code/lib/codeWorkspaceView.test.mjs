import assert from "node:assert/strict";
import test from "node:test";

import {
  codeRuntimePresentation,
  codeThreadLabel,
  groupCodePreparations,
  selectCodeThreadId,
} from "./codeWorkspaceView.ts";

function boundThread(id, overrides = {}) {
  return {
    binding: {
      communityId: "community-1",
      projectDtag: "project-1",
      repositoryIdentity: "a".repeat(64),
      codexThreadId: id,
      executionMode: "worktree",
      executionRoot: `/worktrees/${id}`,
      baseRef: "b".repeat(40),
      worktreeId: id,
    },
    thread: {
      id,
      sessionId: null,
      forkedFromId: null,
      parentThreadId: null,
      preview: null,
      ephemeral: false,
      modelProvider: null,
      createdAt: 1,
      updatedAt: 2,
      cwd: `/worktrees/${id}`,
      name: null,
      status: null,
      turns: [],
      ...overrides,
    },
    unavailable: null,
  };
}

test("runtime copy distinguishes ready, transitional, and recovery states", () => {
  assert.equal(codeRuntimePresentation("ready").tone, "ready");
  assert.equal(codeRuntimePresentation("initializing").tone, "pending");
  assert.equal(codeRuntimePresentation("notInstalled").tone, "error");
  assert.match(codeRuntimePresentation("failed").description, /retry/i);
  assert.equal(codeRuntimePresentation(null).label, "Checking runtime");
});

test("routed thread selection fails closed to an in-scope row", () => {
  const threads = [boundThread("thread-new"), boundThread("thread-old")];
  assert.equal(selectCodeThreadId("thread-old", threads), "thread-old");
  assert.equal(selectCodeThreadId("foreign-thread", threads), "thread-new");
  assert.equal(selectCodeThreadId(null, []), null);
});

test("thread labels prefer name, then preview, then a short opaque id", () => {
  assert.equal(
    codeThreadLabel(boundThread("thread-1", { name: "Fix tests" })),
    "Fix tests",
  );
  assert.equal(
    codeThreadLabel(boundThread("thread-2", { preview: "Update docs" })),
    "Update docs",
  );
  assert.equal(codeThreadLabel(boundThread("abcdefghijk")), "Task abcdefgh");
});

test("preparations remain distinct durable states", () => {
  const base = {
    preparationId: "prepared",
    communityId: "community-1",
    projectDtag: "project-1",
    repositoryIdentity: "a".repeat(64),
    executionMode: "worktree",
    executionRoot: "/worktree",
    baseRef: "b".repeat(40),
    worktreeId: "worktree-1",
  };
  const groups = groupCodePreparations([
    { ...base, state: "prepared" },
    { ...base, preparationId: "starting", state: "starting" },
  ]);
  assert.deepEqual(
    groups.prepared.map(({ preparationId }) => preparationId),
    ["prepared"],
  );
  assert.deepEqual(
    groups.starting.map(({ preparationId }) => preparationId),
    ["starting"],
  );
});
