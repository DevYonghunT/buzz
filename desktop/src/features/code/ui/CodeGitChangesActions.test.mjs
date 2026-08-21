import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { CodeGitChangesActions } from "./CodeGitChangesActions.tsx";

test("working changes expose semantic lanes, exact actions, and partial-stage state", () => {
  const file = {
    fileId: "c".repeat(64),
    path: "src/a very long path/main.ts",
    status: "modified",
    binary: false,
    additions: 2,
    deletions: 1,
    patch: "",
    truncated: false,
  };
  const status = {
    state: "ready",
    runtimeGeneration: 7,
    statusRevision: 8,
    writeGeneration: 2,
    snapshotSequence: 3,
    scope: {
      communityId: "community-1",
      projectDtag: "project-1",
      repositoryIdentity: "a".repeat(64),
    },
    threadId: "thread-1",
    snapshotId: "b".repeat(64),
    headCommit: "d".repeat(40),
    task: {
      files: [file],
      totalFiles: 1,
      filesTruncated: false,
      additions: 2,
      deletions: 1,
    },
    staged: {
      files: [file],
      totalFiles: 1,
      filesTruncated: false,
      additions: 2,
      deletions: 1,
    },
    unstaged: {
      files: [file],
      totalFiles: 1,
      filesTruncated: false,
      additions: 2,
      deletions: 1,
    },
    hasConflicts: false,
    commitIdentity: { name: "Human", email: "human@example.com" },
    capabilities: {
      stage: { enabled: true, reason: null },
      unstage: { enabled: true, reason: null },
      commit: { enabled: true, reason: null },
    },
    blockingReceipt: null,
  };
  const html = renderToStaticMarkup(
    React.createElement(CodeGitChangesActions, {
      busy: false,
      onCommit() {},
      onMutate() {},
      status,
    }),
  );
  assert.match(html, /<section aria-labelledby="code-git-unstage-heading"/);
  assert.match(html, /aria-label="Staged changes"/);
  assert.match(html, /aria-label="Unstage src\/a very long path\/main.ts"/);
  assert.match(html, /aria-label="Stage src\/a very long path\/main.ts"/);
  assert.match(html, /title="src\/a very long path\/main.ts"/);
  assert.match(html, /Partially staged/);
});
