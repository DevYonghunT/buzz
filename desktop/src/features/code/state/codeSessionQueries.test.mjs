import assert from "node:assert/strict";
import test from "node:test";

import {
  codeRepositoryQueryOptions,
  codeRuntimeProbeQueryOptions,
  codeRuntimeStatusQueryOptions,
  codeSessionQueryKeys,
  codeThreadPreparationsQueryOptions,
  codeThreadsQueryOptions,
  codeWorktreeStatusQueryOptions,
} from "./codeSessionQueries.ts";

const scope = {
  communityId: "community-1",
  projectDtag: "project-1",
  repositoryIdentity: "a".repeat(64),
};

const descriptor = {
  executionMode: "local",
  repositoryIdentity: scope.repositoryIdentity,
  executionRoot: "/native/stored-root",
  baseRef: "b".repeat(40),
  worktreeId: null,
};

const repositoryInput = {
  repositoryRoot: "/native/repository",
  baseRef: "main",
};

test("query keys isolate every community, project, and repository coordinate", () => {
  assert.deepEqual(codeSessionQueryKeys.runtimeStatus(), [
    "schoolx-code",
    "runtime",
    "status",
  ]);
  assert.deepEqual(codeSessionQueryKeys.repository(repositoryInput), [
    "schoolx-code",
    "repository",
    "/native/repository",
    "main",
  ]);
  assert.deepEqual(codeSessionQueryKeys.preparations(scope), [
    "schoolx-code",
    "scope",
    "community-1",
    "project-1",
    "a".repeat(64),
    "preparations",
  ]);
  assert.notDeepEqual(
    codeSessionQueryKeys.threads(scope),
    codeSessionQueryKeys.threads({ ...scope, communityId: "community-2" }),
  );
  assert.notDeepEqual(
    codeSessionQueryKeys.threads(scope),
    codeSessionQueryKeys.threads({ ...scope, projectDtag: "project-2" }),
  );
  assert.notDeepEqual(
    codeSessionQueryKeys.threads(scope),
    codeSessionQueryKeys.threads({
      ...scope,
      repositoryIdentity: "c".repeat(64),
    }),
  );
  assert.deepEqual(codeSessionQueryKeys.worktreeStatus(descriptor), [
    "schoolx-code",
    "worktree-status",
    "local",
    scope.repositoryIdentity,
    "/native/stored-root",
    "b".repeat(40),
    null,
  ]);
});

test("query options delegate snapshots to the typed adapter", async () => {
  const calls = [];
  const probe = {
    available: false,
    executable: null,
    version: null,
    error: "missing",
  };
  const status = {
    phase: "stopped",
    generation: 0,
    executable: null,
    version: null,
    pid: null,
    userAgent: null,
    codexHome: null,
    platformFamily: null,
    platformOs: null,
    queuedNotifications: 0,
    lastError: null,
  };
  const repository = {
    repositoryRoot: "/native/repository",
    gitCommonDir: "/native/repository/.git",
    repositoryIdentity: scope.repositoryIdentity,
  };
  const preparations = [];
  const threads = { data: [], nextCursor: null, backwardsCursor: null };
  const worktreeStatus = {
    descriptor,
    headCommit: "b".repeat(40),
    branch: "main",
    dirty: false,
  };
  const api = {
    async probeCodeRuntime() {
      calls.push(["probe"]);
      return probe;
    },
    async getCodeRuntimeStatus() {
      calls.push(["status"]);
      return status;
    },
    async inspectCodeRepository(input) {
      calls.push(["repository", input]);
      return repository;
    },
    async listCodeThreadPreparations(input) {
      calls.push(["preparations", input]);
      return preparations;
    },
    async listCodeThreads(input) {
      calls.push(["threads", input]);
      return threads;
    },
    async getCodeWorktreeStatus(input) {
      calls.push(["worktree", input]);
      return worktreeStatus;
    },
  };

  assert.equal(await codeRuntimeProbeQueryOptions(api).queryFn(), probe);
  assert.equal(await codeRuntimeStatusQueryOptions(api).queryFn(), status);
  assert.equal(
    await codeRepositoryQueryOptions(repositoryInput, api).queryFn(),
    repository,
  );
  assert.equal(
    await codeThreadPreparationsQueryOptions(scope, api).queryFn(),
    preparations,
  );
  assert.equal(await codeThreadsQueryOptions(scope, api).queryFn(), threads);
  assert.equal(
    await codeWorktreeStatusQueryOptions(descriptor, api).queryFn(),
    worktreeStatus,
  );
  assert.deepEqual(calls, [
    ["probe"],
    ["status"],
    ["repository", repositoryInput],
    ["preparations", { scope }],
    ["threads", { scope }],
    ["worktree", descriptor],
  ]);
});
