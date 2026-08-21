import assert from "node:assert/strict";
import test from "node:test";

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { codeSessionQueryKeys } from "../state/codeSessionQueries.ts";
import {
  codeWorktreeBlockerLabel,
  CodeWorktreeInventorySection,
} from "./CodeWorktreeInventorySection.tsx";

const scope = {
  communityId: "community-1",
  projectDtag: "project-1",
  repositoryIdentity: "a".repeat(64),
};
const baseRef = "b".repeat(40);
const descriptor = {
  executionMode: "worktree",
  repositoryIdentity: scope.repositoryIdentity,
  executionRoot: "/native/preserved-root",
  baseRef,
  worktreeId: "worktree-1",
};

function renderInventory(rows, actionsReady = true) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  queryClient.setQueryData(codeSessionQueryKeys.worktrees(scope), rows);
  try {
    return renderToStaticMarkup(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(CodeWorktreeInventorySection, {
          actionsReady,
          scope,
        }),
      ),
    );
  } finally {
    queryClient.clear();
  }
}

async function renderInventoryError(message) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  await assert.rejects(
    queryClient.fetchQuery({
      queryKey: codeSessionQueryKeys.worktrees(scope),
      queryFn: async () => {
        throw new Error(message);
      },
    }),
  );
  try {
    return renderToStaticMarkup(
      React.createElement(
        QueryClientProvider,
        { client: queryClient },
        React.createElement(CodeWorktreeInventorySection, {
          actionsReady: true,
          scope,
        }),
      ),
    );
  } finally {
    queryClient.clear();
  }
}

test("closed inventory blockers have stable user-facing labels", () => {
  assert.deepEqual(
    [
      "activeBinding",
      "lifecycleUnsettled",
      "unfinishedPreparation",
      "localCheckout",
      "unavailableRoot",
      "dirtyRoot",
      "branchAttached",
      "headDrift",
      "mergeProofUnavailable",
    ].map(codeWorktreeBlockerLabel),
    [
      "Active task",
      "Task lifecycle is unsettled",
      "Unfinished task preparation",
      "Local checkout",
      "Worktree unavailable",
      "Uncommitted changes",
      "Branch is attached",
      "HEAD differs from its immutable base",
      "Merge proof unavailable",
    ],
  );
});

test("inventory renders removal only for an eligible archived binding", () => {
  const html = renderInventory([
    {
      scope,
      authority: {
        type: "binding",
        threadId: "thread-active",
        lifecycle: "active",
      },
      descriptor,
      inspection: {
        status: "available",
        headCommit: baseRef,
        branch: null,
        dirty: false,
      },
      preserved: true,
      canRemove: false,
      blockers: ["activeBinding"],
    },
    {
      scope,
      authority: {
        type: "binding",
        threadId: "thread-archived",
        lifecycle: "archived",
      },
      descriptor: {
        ...descriptor,
        executionRoot: "/native/missing-root",
        worktreeId: "worktree-2",
      },
      inspection: { status: "unavailable", error: "Root is missing" },
      preserved: true,
      canRemove: false,
      blockers: ["unavailableRoot", "mergeProofUnavailable"],
    },
    {
      scope,
      authority: {
        type: "binding",
        threadId: "thread-removable",
        lifecycle: "archived",
      },
      descriptor: {
        ...descriptor,
        executionRoot: "/native/removable-root",
        worktreeId: "worktree-3",
      },
      inspection: {
        status: "available",
        headCommit: "c".repeat(40),
        branch: null,
        dirty: false,
      },
      preserved: true,
      canRemove: true,
      blockers: [],
    },
    {
      scope,
      authority: {
        type: "binding",
        threadId: "thread-removable-peer",
        lifecycle: "archived",
      },
      descriptor: {
        ...descriptor,
        executionRoot: "/native/removable-peer-root",
        worktreeId: "worktree-4",
      },
      inspection: {
        status: "available",
        headCommit: "d".repeat(40),
        branch: null,
        dirty: false,
      },
      preserved: true,
      canRemove: true,
      blockers: [],
    },
  ]);

  assert.match(html, />Managed worktrees</);
  assert.equal((html.match(/>Preserved</g) ?? []).length, 2);
  assert.equal((html.match(/>Ready to remove</g) ?? []).length, 2);
  assert.match(html, />Active task</);
  assert.match(html, />Worktree unavailable</);
  assert.match(html, />Merge proof unavailable</);
  assert.match(html, /Root is missing/);
  assert.match(html, /aria-label="Refresh managed worktrees"/);
  assert.equal((html.match(/<button\b/g) ?? []).length, 3);
  assert.match(html, />Remove worktree</);
  assert.match(html, /data-testid="code-worktree-remove-thread-removable"/);
  assert.match(html, /aria-label="Remove worktree for task thread-removable"/);
  assert.match(
    html,
    /aria-label="Remove worktree for task thread-removable-peer"/,
  );
  assert.doesNotMatch(html, /data-testid="code-worktree-remove-dialog"/);
});

test("eligible removal remains visible but disabled while actions sync", () => {
  const html = renderInventory(
    [
      {
        scope,
        authority: {
          type: "binding",
          threadId: "thread-removable",
          lifecycle: "archived",
        },
        descriptor,
        inspection: {
          status: "available",
          headCommit: baseRef,
          branch: null,
          dirty: false,
        },
        preserved: true,
        canRemove: true,
        blockers: [],
      },
    ],
    false,
  );
  assert.match(html, /data-testid="code-worktree-remove-thread-removable"/);
  assert.match(
    html,
    /data-testid="code-worktree-remove-thread-removable"[^>]*disabled=""/,
  );
});

test("inventory empty state points to managed task creation", () => {
  const html = renderInventory([]);
  assert.match(
    html,
    /Create a managed task to see its preserved worktree here\./,
  );
  assert.equal((html.match(/<button\b/g) ?? []).length, 1);
  assert.doesNotMatch(html, />Remove</);
});

test("inventory keeps refresh and inline retry available after an error", async () => {
  const html = await renderInventoryError("Inventory read failed");
  assert.match(html, /role="alert"/);
  assert.match(html, /Inventory read failed/);
  assert.match(html, />Retry inventory</);
  assert.match(html, /aria-label="Refresh managed worktrees"/);
  assert.equal((html.match(/<button\b/g) ?? []).length, 2);
  assert.doesNotMatch(html, />Remove</);
});
