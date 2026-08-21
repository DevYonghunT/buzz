import assert from "node:assert/strict";
import test from "node:test";

import {
  CodeGitAcknowledgeInputSchema,
  CodeGitCommitInputSchema,
  CodeGitIndexMutationInputSchema,
  CodeGitStatusSchema,
} from "./codeGitSchemas.ts";

const scope = {
  communityId: "community-1",
  projectDtag: "project-1",
  repositoryIdentity: "a".repeat(64),
};
const snapshotId = "b".repeat(64);
const fileId = "c".repeat(64);

test("Git mutation inputs accept opaque coordinates and reject caller Git authority", () => {
  const input = {
    scope,
    threadId: "thread-1",
    writeGeneration: 2,
    snapshotId,
    fileId,
  };
  assert.deepEqual(CodeGitIndexMutationInputSchema.parse(input), input);
  for (const [field, value] of [
    ["path", "src/main.ts"],
    ["cwd", "/caller/root"],
    ["ref", "refs/heads/main"],
    ["oid", "d".repeat(40)],
    ["operationId", "e".repeat(64)],
    ["argv", ["add", "."]],
    ["identity", { name: "Caller", email: "caller@example.com" }],
    ["force", true],
  ]) {
    assert.equal(
      CodeGitIndexMutationInputSchema.safeParse({ ...input, [field]: value })
        .success,
      false,
      `accepted caller Git authority ${field}`,
    );
  }
  assert.equal(
    CodeGitCommitInputSchema.safeParse({
      ...input,
      message: "Commit",
      path: "src/main.ts",
    }).success,
    false,
  );
  assert.equal(
    CodeGitAcknowledgeInputSchema.safeParse({
      scope,
      threadId: "thread-1",
      operationId: "e".repeat(64),
      writeGeneration: 3,
      snapshotId,
    }).success,
    true,
  );
});

test("ready status enforces complete sorted action lanes and shared partial-stage file IDs", () => {
  const file = {
    fileId,
    path: "src/main.ts",
    status: "modified",
    binary: false,
    additions: 1,
    deletions: 1,
    patch: "@@ -1 +1 @@\n-old\n+new",
    truncated: false,
  };
  const set = {
    files: [file],
    totalFiles: 1,
    filesTruncated: false,
    additions: 1,
    deletions: 1,
  };
  const status = {
    state: "ready",
    runtimeGeneration: 7,
    statusRevision: 8,
    writeGeneration: 2,
    snapshotSequence: 3,
    scope,
    threadId: "thread-1",
    snapshotId,
    headCommit: "d".repeat(40),
    task: set,
    staged: set,
    unstaged: set,
    hasConflicts: false,
    commitIdentity: { name: "Human", email: "human@example.com" },
    capabilities: {
      stage: { enabled: true, reason: null },
      unstage: { enabled: true, reason: null },
      commit: { enabled: true, reason: null },
    },
    blockingReceipt: null,
  };
  assert.equal(CodeGitStatusSchema.safeParse(status).success, true);
  assert.equal(
    CodeGitStatusSchema.safeParse({
      ...status,
      unstaged: {
        ...set,
        files: [{ ...file, fileId: "e".repeat(64) }],
      },
    }).success,
    false,
  );
  assert.equal(
    CodeGitStatusSchema.safeParse({
      ...status,
      staged: { ...set, filesTruncated: true },
    }).success,
    false,
  );
});
