import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient } from "@tanstack/react-query";

import {
  codeModelsQueryOptions,
  codeRepositoryQueryOptions,
  codeRuntimeProbeQueryOptions,
  codeRuntimeStatusQueryOptions,
  codeSessionQueryKeys,
  codeThreadChangesQueryOptions,
  codeThreadGitAttemptQueryOptions,
  codeThreadPreparationsQueryOptions,
  codeThreadsQueryOptions,
  codeWorktreeStatusQueryOptions,
  codeWorktreesQueryOptions,
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
  assert.deepEqual(codeSessionQueryKeys.models(7), [
    "schoolx-code",
    "runtime",
    "models",
    7,
  ]);
  assert.notDeepEqual(
    codeSessionQueryKeys.models(7),
    codeSessionQueryKeys.models(8),
  );
  assert.deepEqual(
    codeSessionQueryKeys.threadGitAttempt({
      scope,
      threadId: "thread-1",
    }),
    [
      "schoolx-code",
      "scope",
      "community-1",
      "project-1",
      "a".repeat(64),
      "thread-git-attempt",
      "thread-1",
    ],
  );
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
  assert.deepEqual(codeSessionQueryKeys.worktrees(scope), [
    "schoolx-code",
    "scope",
    "community-1",
    "project-1",
    "a".repeat(64),
    "worktrees",
  ]);
  assert.deepEqual(codeSessionQueryKeys.worktreeRemovalAttempt(scope), [
    "schoolx-code",
    "scope",
    "community-1",
    "project-1",
    "a".repeat(64),
    "worktree-removal-attempt",
  ]);
  assert.notDeepEqual(
    codeSessionQueryKeys.worktrees(scope),
    codeSessionQueryKeys.worktrees({ ...scope, projectDtag: "project-2" }),
  );
  assert.notDeepEqual(
    codeSessionQueryKeys.threads(scope),
    codeSessionQueryKeys.threads({ ...scope, communityId: "community-2" }),
  );
  assert.deepEqual(
    codeSessionQueryKeys.threadChanges({
      scope,
      threadId: "thread-1",
      runtimeGeneration: 7,
    }),
    [
      "schoolx-code",
      "scope",
      "community-1",
      "project-1",
      "a".repeat(64),
      "thread-changes",
      "thread-1",
      7,
    ],
  );
  assert.notDeepEqual(
    codeSessionQueryKeys.threadChanges({
      scope,
      threadId: "thread-1",
      runtimeGeneration: 7,
    }),
    codeSessionQueryKeys.threadChanges({
      scope,
      threadId: "thread-2",
      runtimeGeneration: 7,
    }),
  );
  assert.notDeepEqual(
    codeSessionQueryKeys.threadChanges({
      scope,
      threadId: "thread-1",
      runtimeGeneration: 7,
    }),
    codeSessionQueryKeys.threadChanges({
      scope,
      threadId: "thread-1",
      runtimeGeneration: 8,
    }),
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

test("Git handoff attempt cache is generation-independent and never expires", () => {
  const options = codeThreadGitAttemptQueryOptions({
    scope,
    threadId: "thread-1",
  });

  assert.equal(options.gcTime, Number.POSITIVE_INFINITY);
  assert.equal(options.staleTime, Number.POSITIVE_INFINITY);
  assert.equal(options.enabled, false);
  assert.doesNotMatch(JSON.stringify(options.queryKey), /runtime|generation/i);
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
  const models = {
    runtimeGeneration: 7,
    models: [],
    recentSelection: null,
  };
  const preparations = [];
  const worktrees = [];
  const threads = { data: [], nextCursor: null, backwardsCursor: null };
  const changes = {
    files: [],
    additions: 0,
    deletions: 0,
    commitBody: null,
    totalFiles: 0,
    filesTruncated: false,
  };
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
    async listCodeModels() {
      calls.push(["models"]);
      return models;
    },
    async inspectCodeRepository(input) {
      calls.push(["repository", input]);
      return repository;
    },
    async listCodeThreadPreparations(input) {
      calls.push(["preparations", input]);
      return preparations;
    },
    async listCodeWorktrees(input) {
      calls.push(["worktrees", input]);
      return worktrees;
    },
    async listCodeThreads(input) {
      calls.push(["threads", input]);
      return threads;
    },
    async getCodeThreadChanges(input) {
      calls.push(["changes", input]);
      return changes;
    },
    async getCodeWorktreeStatus(input) {
      calls.push(["worktree", input]);
      return worktreeStatus;
    },
  };

  assert.equal(await codeRuntimeProbeQueryOptions(api).queryFn(), probe);
  assert.equal(await codeRuntimeStatusQueryOptions(api).queryFn(), status);
  assert.equal(await codeModelsQueryOptions(7, api).queryFn(), models);
  assert.equal(
    await codeRepositoryQueryOptions(repositoryInput, api).queryFn(),
    repository,
  );
  assert.equal(
    await codeThreadPreparationsQueryOptions(scope, api).queryFn(),
    preparations,
  );
  assert.equal(
    await codeWorktreesQueryOptions(scope, api).queryFn(),
    worktrees,
  );
  assert.equal(await codeThreadsQueryOptions(scope, api).queryFn(), threads);
  assert.equal(
    await codeThreadChangesQueryOptions(
      { scope, threadId: "thread-1", runtimeGeneration: 7 },
      api,
    ).queryFn(),
    changes,
  );
  assert.equal(
    await codeWorktreeStatusQueryOptions(descriptor, api).queryFn(),
    worktreeStatus,
  );
  assert.deepEqual(calls, [
    ["probe"],
    ["status"],
    ["models"],
    ["repository", repositoryInput],
    ["preparations", { scope }],
    ["worktrees", { scope }],
    ["threads", { scope }],
    ["changes", { scope, threadId: "thread-1" }],
    ["worktree", descriptor],
  ]);
});

test("model catalog rejects a response from a different runtime generation", async () => {
  await assert.rejects(
    codeModelsQueryOptions(7, {
      async listCodeModels() {
        return { runtimeGeneration: 8, models: [], recentSelection: null };
      },
    }).queryFn(),
    /must match the requested runtime generation/,
  );
});

test("thread changes read once per generation and invalidate exactly", async () => {
  const calls = [];
  const changes = {
    files: [],
    additions: 0,
    deletions: 0,
    commitBody: null,
    totalFiles: 0,
    filesTruncated: false,
  };
  const api = {
    async getCodeThreadChanges(input) {
      calls.push(input);
      return changes;
    },
  };
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const generationSeven = {
    scope,
    threadId: "thread-1",
    runtimeGeneration: 7,
  };
  const generationEight = {
    ...generationSeven,
    runtimeGeneration: 8,
  };
  const fetchChanges = (identity) =>
    queryClient.fetchQuery({
      ...codeThreadChangesQueryOptions(identity, api),
      staleTime: 1_000,
    });

  try {
    await fetchChanges(generationSeven);
    await fetchChanges(generationSeven);
    assert.equal(calls.length, 1);

    await fetchChanges(generationEight);
    await fetchChanges(generationEight);
    assert.equal(calls.length, 2);

    await queryClient.invalidateQueries({
      exact: true,
      queryKey: codeSessionQueryKeys.threadChanges(generationEight),
      refetchType: "none",
    });
    await fetchChanges(generationEight);
    await fetchChanges(generationSeven);

    assert.equal(calls.length, 3);
    assert.deepEqual(calls, [
      { scope, threadId: "thread-1" },
      { scope, threadId: "thread-1" },
      { scope, threadId: "thread-1" },
    ]);
  } finally {
    queryClient.clear();
  }
});
