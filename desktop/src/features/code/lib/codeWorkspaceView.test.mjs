import assert from "node:assert/strict";
import test from "node:test";

import {
  codeRuntimePresentation,
  codeThreadLifecycleCapabilities,
  codeThreadLifecycleLabel,
  codeThreadLabel,
  codeThreadMatchesSearch,
  codeThreadPreparationLabels,
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
    lifecycle: "active",
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
  const archived = {
    ...boundThread("thread-old"),
    lifecycle: "archived",
  };
  const threads = [boundThread("thread-new"), archived];
  assert.equal(selectCodeThreadId("thread-old", threads), "thread-old");
  assert.equal(selectCodeThreadId("foreign-thread", threads), "thread-new");
  assert.equal(selectCodeThreadId(null, []), null);
});

test("thread lifecycle capabilities fail closed outside stable states", () => {
  assert.deepEqual(codeThreadLifecycleCapabilities("active"), {
    canArchive: true,
    canExecute: true,
    canFork: true,
    canReadChanges: true,
    canRename: true,
    canUnarchive: false,
    stable: true,
  });
  assert.deepEqual(codeThreadLifecycleCapabilities("archived"), {
    canArchive: false,
    canExecute: false,
    canFork: false,
    canReadChanges: true,
    canRename: true,
    canUnarchive: true,
    stable: true,
  });
  for (const lifecycle of ["archiving", "unarchiving", "unknown"]) {
    assert.deepEqual(codeThreadLifecycleCapabilities(lifecycle), {
      canArchive: false,
      canExecute: false,
      canFork: false,
      canReadChanges: false,
      canRename: false,
      canUnarchive: false,
      stable: false,
    });
  }
  assert.equal(codeThreadLifecycleLabel("archived"), "Archived");
  assert.equal(codeThreadLifecycleLabel("unknown"), "Status unknown");
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

test("local thread search matches names, previews, and opaque ids", () => {
  const named = boundThread("0198fabc-thread-name", {
    name: "Repair Terminal Lifecycle",
    preview: "Keep the hidden shell alive",
  });
  const previewed = boundThread("0198fdef-thread-preview", {
    name: null,
    preview: "Review native ownership",
  });

  assert.equal(codeThreadMatchesSearch(named, "  TERMINAL  "), true);
  assert.equal(codeThreadMatchesSearch(named, "hidden shell"), true);
  assert.equal(codeThreadMatchesSearch(previewed, "native owner"), true);
  assert.equal(codeThreadMatchesSearch(named, "0198fabc-thread-name"), true);
  assert.equal(codeThreadMatchesSearch(previewed, "0198fdef"), true);
  assert.equal(codeThreadMatchesSearch(named, ""), true);
  assert.equal(codeThreadMatchesSearch(named, "archive"), false);
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
    operation: "start",
    sourceThreadId: null,
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

test("preparation labels distinguish all start and fork recovery actions", () => {
  assert.deepEqual(
    [
      ["start", "prepared"],
      ["start", "starting"],
      ["fork", "prepared"],
      ["fork", "starting"],
    ].map(([operation, state]) =>
      codeThreadPreparationLabels({ operation, state }),
    ),
    [
      { action: "Start task", title: "Prepared task" },
      { action: "Recover task", title: "Needs recovery" },
      { action: "Continue fork", title: "Prepared fork" },
      { action: "Recover fork", title: "Fork needs recovery" },
    ],
  );
});
