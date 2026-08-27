import assert from "node:assert/strict";
import test from "node:test";

import {
  defaultCodeTaskExecutionMode,
  localCheckoutSnapshotChanged,
  localCheckoutSnapshotFromPreparation,
  localCheckoutSnapshotFromStatus,
  supportsManagedCodeWorktrees,
} from "./codeTaskCreation.ts";

test("new Code tasks use the execution mode supported by the desktop OS", () => {
  assert.equal(defaultCodeTaskExecutionMode("MacIntel"), "worktree");
  assert.equal(defaultCodeTaskExecutionMode("Linux x86_64"), "worktree");
  assert.equal(defaultCodeTaskExecutionMode("Win32"), "local");
  assert.equal(supportsManagedCodeWorktrees("Win32"), false);
});

test("local checkout display state excludes native path and Git authority", () => {
  const prepared = {
    preparationId: "preparation-1",
    scope: {
      communityId: "community-1",
      projectDtag: "project-1",
      repositoryIdentity: "a".repeat(64),
    },
    worktree: {
      repository: {
        repositoryRoot: "/native/repository",
        gitCommonDir: "/native/repository/.git",
        repositoryIdentity: "a".repeat(64),
      },
      descriptor: {
        executionMode: "local",
        repositoryIdentity: "a".repeat(64),
        executionRoot: "/native/repository",
        baseRef: "b".repeat(40),
        worktreeId: null,
      },
      headCommit: "c".repeat(40),
      branch: "feature/local-ui",
      dirty: true,
    },
  };

  assert.deepEqual(localCheckoutSnapshotFromPreparation(prepared), {
    branch: "feature/local-ui",
    dirty: true,
  });
  assert.deepEqual(
    Object.keys(localCheckoutSnapshotFromPreparation(prepared)),
    ["branch", "dirty"],
  );
});

test("native branch or dirty-state drift requires another confirmation", () => {
  const reviewed = { branch: "main", dirty: false };
  assert.equal(
    localCheckoutSnapshotChanged(
      reviewed,
      localCheckoutSnapshotFromStatus({
        descriptor: {
          executionMode: "local",
          repositoryIdentity: "a".repeat(64),
          executionRoot: "/native/repository",
          baseRef: "b".repeat(40),
          worktreeId: null,
        },
        headCommit: "c".repeat(40),
        branch: "main",
        dirty: false,
      }),
    ),
    false,
  );
  assert.equal(
    localCheckoutSnapshotChanged(reviewed, {
      branch: "feature/drifted",
      dirty: false,
    }),
    true,
  );
  assert.equal(
    localCheckoutSnapshotChanged(reviewed, { branch: "main", dirty: true }),
    true,
  );
});
