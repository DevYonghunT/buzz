import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { ProjectDiffFilesPanel } from "@/features/projects/ui/ProjectPullRequestFilesChangedPanel.tsx";
import { codeSessionQueryKeys } from "../state/codeSessionQueries.ts";
import { CodeChangesPanel } from "./CodeChangesPanel.tsx";

const scope = {
  communityId: "community-1",
  projectDtag: "project-1",
  repositoryIdentity: "a".repeat(64),
};
const runtimeGeneration = 7;
const binding = {
  ...scope,
  codexThreadId: "thread-1",
  executionMode: "worktree",
  executionRoot: "/native/stored-root",
  baseRef: "b".repeat(40),
  worktreeId: "worktree-1",
};

function gitController(overrides = {}) {
  return {
    attempt: null,
    busy: false,
    async commit() {
      return null;
    },
    commitPending: false,
    gitBlockerReason: null,
    operationPending: false,
    query: {
      error: null,
      isError: false,
      isFetching: false,
      isLoading: false,
      isPending: false,
      async refetch() {},
    },
    ready: null,
    async reconcile() {},
    async retryStatus() {},
    async runIndexMutation() {},
    status: {
      state: "blocked",
      reason: "Read-only fixture",
      remediation: "Use the legacy diff snapshot.",
    },
    ...overrides,
  };
}

function renderChangesPanel({ changes, controller, enabled }) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  if (changes) {
    queryClient.setQueryData(
      codeSessionQueryKeys.threadChanges({
        scope,
        threadId: binding.codexThreadId,
        runtimeGeneration,
      }),
      changes,
    );
  }
  try {
    return renderToStaticMarkup(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(CodeChangesPanel, {
          binding,
          controller: controller ?? gitController(),
          enabled,
          onClose() {},
          runtimeGeneration,
          scope,
        }),
      ),
    );
  } finally {
    queryClient.clear();
  }
}

test("disabled Changes waits for synchronization and disables refresh", () => {
  const html = renderChangesPanel({ changes: null, enabled: false });

  assert.match(html, /data-testid="code-changes-sync-pending"/);
  assert.match(html, /Waiting for Code activity synchronization…/);
  assert.match(html, /aria-label="Refresh changed files"[^>]*disabled=""/);
  assert.doesNotMatch(html, /Loading changed files…/);
});

test("Git status transport errors expose an explicit Retry action", () => {
  const html = renderChangesPanel({
    changes: null,
    controller: gitController({
      query: {
        error: new Error("invalid status payload"),
        isError: true,
        isFetching: false,
        isLoading: false,
        isPending: false,
        async refetch() {},
      },
      status: undefined,
    }),
    enabled: true,
  });

  assert.match(html, /role="alert"/);
  assert.match(html, /Git write status could not be verified/);
  assert.match(html, />Retry Git status</);
});

test("Changes surfaces aggregate list and patch completeness plus file metadata", () => {
  const html = renderChangesPanel({
    enabled: true,
    changes: {
      files: [
        {
          path: "assets/icon.png",
          status: "modified",
          binary: true,
          additions: 0,
          deletions: 0,
          patch: "",
          truncated: false,
        },
        {
          path: "src/large.ts",
          status: "added",
          binary: false,
          additions: 1,
          deletions: 0,
          patch: "@@ -0,0 +1 @@\n+export {};",
          truncated: true,
        },
      ],
      additions: 1,
      deletions: 0,
      commitBody: null,
      totalFiles: 3,
      filesTruncated: true,
    },
  });

  assert.match(html, /data-testid="code-changes-completeness-warning"/);
  assert.match(
    html,
    /Showing 2 of 3 changed files\. Review the local checkout for the complete file list\. Addition and deletion totals cover the shown files only\./,
  );
  assert.match(
    html,
    /Among the shown files, 1 file patch truncated\. Review the local checkout for the complete diff\./,
  );
  assert.match(html, /aria-label="Modified file status"/);
  assert.match(html, />Modified</);
  assert.match(html, />Binary</);
  assert.match(html, /Binary file preview is not available\./);
});

test("the shared diff panel preserves legacy project rows without metadata", () => {
  const html = renderToStaticMarkup(
    React.createElement(ProjectDiffFilesPanel, {
      diff: {
        files: [
          {
            path: "src/legacy.ts",
            additions: 3,
            deletions: 2,
            patch: "@@ -1 +1 @@\n-old\n+new",
            truncated: false,
          },
        ],
        additions: 3,
        deletions: 2,
        commitBody: null,
      },
      embedded: true,
      error: null,
      headerLabel: "Legacy project diff",
      isLoading: false,
      subjectLabel: "pull request",
    }),
  );

  assert.match(html, /legacy\.ts/);
  assert.match(html, /\+3/);
  assert.match(html, /-2/);
  assert.doesNotMatch(html, /file status/);
  assert.doesNotMatch(html, /Binary file preview is not available\./);
});
